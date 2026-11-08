"""Keep Lumo's numeric date version aligned; used by package.ps1. No publishing."""
import argparse
from datetime import date, datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parent.parent
FILES = ("Cargo.toml", "Cargo.lock", "crates/lumo/res/lumo.rc",
         "crates/lumo/res/lumo.manifest", "installer/lumo.nsi", "installer/lumo.iss")
STATE = ".tools/package-state.json"


def validate(version):
    if not re.fullmatch(r"[1-9]\d*\.(?:0|[1-9]\d*)\.[1-9]\d*", version):
        raise ValueError("Use year.monthday.build without leading zeros; build starts at 1")
    year, monthday, build = map(int, version.split('.'))
    if any(n > 65535 for n in (year, monthday, build)):
        raise ValueError("Each version field must fit 0–65535")
    date(year, monthday // 100, monthday % 100)
    return year, monthday, build


def next_version(current, today):
    year, monthday, build = validate(current)
    previous = date(year, monthday // 100, monthday % 100)
    if today < previous:
        raise ValueError("Build date is before the current version; use -KeepVersion to rebuild it")
    result = f"{today.year}.{today.month * 100 + today.day}.{build + 1 if today == previous else 1}"
    validate(result)
    return result


def read_files(root):
    return {name: (root / name).read_bytes().decode('utf-8') for name in FILES}


def current_version(contents):
    matches = re.findall(r'(?m)^version = "([^"]+)"', contents["Cargo.toml"])
    if len(matches) != 1:
        raise ValueError("Could not locate the workspace version")
    validate(matches[0])
    return matches[0]


def render(contents, version):
    """Prepare every replacement before writing any file; preserve line endings."""
    output = dict(contents)
    def replace(name, pattern, replacement, expected):
        output[name], count = re.subn(pattern, replacement, output[name])
        if count != expected:
            raise ValueError(f"Expected {expected} version fields in {name}, found {count}")
    dotted = version + '.0'
    comma = dotted.replace('.', ',')
    quoted = lambda m: m[1] + version + m[2]
    replace("Cargo.toml", r'(?m)^(version = ")[^"]+(\")', quoted, 1)
    replace("Cargo.lock", r'(?m)^(\[\[package\]\]\r?\nname = "(?:lumo|lumo-core)"\r?\nversion = ")[^"]+(\")', quoted, 2)
    replace("crates/lumo/res/lumo.rc", r'(?m)^((?:FILEVERSION|PRODUCTVERSION) )\d+,\d+,\d+,\d+', lambda m: m[1] + comma, 2)
    replace("crates/lumo/res/lumo.rc", r'(?m)^(\s*VALUE "(?:FileVersion|ProductVersion)", ")\d+\.\d+\.\d+(\\0")', quoted, 2)
    replace("crates/lumo/res/lumo.manifest", r'(<assemblyIdentity\b[^>]*\bversion=")[^"]+(\")', lambda m: m[1] + dotted + m[2], 1)
    replace("installer/lumo.nsi", r'(?m)^(!define VERSION ")[^"]+(\")', quoted, 1)
    replace("installer/lumo.nsi", r'(?m)^(VIProductVersion ")[^"]+(\")', lambda m: m[1] + dotted + m[2], 1)
    replace("installer/lumo.iss", r'(?m)^(\s*#define AppVersion ")[^"]+(\")', quoted, 1)
    return output


def check(root):
    contents = read_files(root)
    version = current_version(contents)
    expected = render(contents, version)
    wrong = [name for name in FILES if expected[name] != contents[name]]
    if wrong:
        raise ValueError("Version fields are not aligned: " + ', '.join(wrong))
    return version


def atomic_write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.' + uuid.uuid4().hex + '.tmp')
    try:
        temporary.write_bytes(data)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def set_version(root, version):
    validate(version)
    before = read_files(root)
    after = render(before, version)
    written = []
    try:
        for name in FILES:
            if after[name] != before[name]:
                atomic_write(root / name, after[name].encode('utf-8'))
                written.append(name)
    except OSError:
        for name in written:
            atomic_write(root / name, before[name].encode('utf-8'))
        raise


def source_fingerprint(root):
    """Version-only edits and release notes don't cause another version bump."""
    normalized = render(read_files(root), "0.0.0")
    try:
        result = subprocess.run(['git', '-C', str(root), 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
                                capture_output=True, check=True)
        names = set(result.stdout.decode('utf-8').split('\0'))
    except (OSError, subprocess.CalledProcessError):
        names = {p.relative_to(root).as_posix() for folder in ('crates', 'installer', 'tools')
                 if (root / folder).exists() for p in (root / folder).rglob('*') if p.is_file()}
        names.update(FILES)
    digest = hashlib.sha256()
    for name in sorted(names):
        if not name or name.startswith(('docs/release-notes/', '.tools/', 'dist/', 'target/')) or '__pycache__' in name:
            continue
        path = root / name
        if not path.is_file():
            continue
        data = normalized[name].encode('utf-8') if name in normalized else path.read_bytes()
        digest.update(name.encode('utf-8') + b'\0' + hashlib.sha256(data).digest())
    return digest.hexdigest()


def plan(root, today, version=None, keep=False, bump=False):
    current = check(root)
    fingerprint = source_fingerprint(root)
    try:
        previous = json.loads((root / STATE).read_text())
    except (OSError, ValueError):
        previous = {}
    if version:
        selected = version
    elif keep or (not bump and previous.get('fingerprint') == fingerprint and previous.get('version') == current):
        selected = current
    else:
        selected = next_version(current, today)
    validate(selected)
    set_version(root, selected)
    notes = root / f'docs/release-notes/v{selected}.md'
    if not notes.exists():
        atomic_write(notes, (f'# Lumo {selected}\n\nLocal build prepared by tools/package.ps1. '
                             'Review and complete these notes before publishing.\n').encode())
    state = {'version': selected, 'fingerprint': fingerprint, 'status': 'building'}
    atomic_write(root / STATE, (json.dumps(state, indent=2) + '\n').encode())
    return state


def verify(root, complete=False):
    state = json.loads((root / STATE).read_text())
    if check(root) != state['version'] or source_fingerprint(root) != state['fingerprint']:
        raise ValueError("Sources changed during packaging. Rerun package.ps1 to build the current files")
    if complete:
        state['status'] = 'complete'
        atomic_write(root / STATE, (json.dumps(state, indent=2) + '\n').encode())
    return state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('check', 'plan', 'verify', 'complete'))
    parser.add_argument('--root', type=Path, default=ROOT)
    choices = parser.add_mutually_exclusive_group()
    choices.add_argument('--version')
    choices.add_argument('--keep-version', action='store_true')
    choices.add_argument('--bump-version', action='store_true')
    parser.add_argument('--date', type=date.fromisoformat)
    args = parser.parse_args()
    try:
        if args.action == 'check':
            result = {'version': check(args.root)}
        elif args.action == 'plan':
            today = args.date or datetime.now(timezone(timedelta(hours=5, minutes=30))).date()
            result = plan(args.root, today, args.version, args.keep_version, args.bump_version)
        else:
            result = verify(args.root, args.action == 'complete')
        print(json.dumps(result))
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, f"Version check failed: {error}\n")


if __name__ == '__main__':
    main()

"""Generate/submit manifests from existing, checksum-verified GitHub releases.

Never builds Lumo, changes its version, or creates an app release. Generating
is read-only on GitHub; publishing requires an explicit subcommand and token.
"""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
from urllib.error import HTTPError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen
from zipfile import ZipFile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from version import validate

SOURCE = 'mthlpbs/Lumo'
PACKAGE_ID = 'mthlpbs.Lumo'
SCHEMA = '1.6.0'
MAX_ASSET = 64 * 1024 * 1024


def repository(value):
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*', value):
        raise ValueError('Expected a GitHub owner/repository name')
    return value


def release_version(tag):
    value = tag.removeprefix('v')
    validate(value)
    if tag not in (value, 'v' + value):
        raise ValueError('Expected a numeric app release tag')
    return value


def download(url, limit=MAX_ASSET):
    # Published Lumo downloads are public. Do not forward API credentials to
    # GitHub's redirected release-asset host.
    request = Request(url, headers={'User-Agent': 'Lumo-package-managers'})
    with urlopen(request, timeout=60) as response:
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f'Download exceeds {limit} bytes: {url}')
    return data


def github(path, token='', method='GET', data=None, missing=False):
    headers = {'Accept': 'application/vnd.github+json',
               'X-GitHub-Api-Version': '2022-11-28', 'User-Agent': 'Lumo-package-managers'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    body = None if data is None else json.dumps(data).encode()
    request = Request('https://api.github.com/' + path, data=body, headers=headers, method=method)
    try:
        with urlopen(request, timeout=60) as response:
            return json.load(response)
    except HTTPError as error:
        if missing and error.code == 404:
            return None
        raise ValueError(f'GitHub HTTP {error.code}: {path}. Check repository/token permissions.') from None


def checksums(data):
    result = {}
    for line in data.decode('utf-8-sig').splitlines():
        match = re.fullmatch(r'([0-9a-fA-F]{64})\s+\*?([^/\\\s]+)', line.strip())
        if not match or match[2] in result:
            raise ValueError('Invalid or duplicate SHA256SUMS.txt entry')
        result[match[2]] = match[1].lower()
    return result


def release_assets(release, tag, fetch=download):
    version = release_version(tag)
    if release.get('tag_name') != tag or release.get('draft') or release.get('prerelease'):
        raise ValueError('Package managers require an exact, published stable app release')
    names = [f'Lumo-Setup-{version}.exe', f'Lumo-{version}-portable.zip', 'SHA256SUMS.txt']
    selected = {}
    for name in names:
        matches = [a for a in release['assets'] if a['name'] == name]
        if len(matches) != 1:
            raise ValueError(f'Release must contain exactly one {name}')
        asset = matches[0]
        url = f'https://github.com/{SOURCE}/releases/download/{quote(tag, safe="")}/{name}'
        if asset['browser_download_url'].lower() != url.lower():
            raise ValueError(f'Unexpected download host/path for {name}')
        selected[name] = asset['browser_download_url']
    expected = checksums(fetch(selected[names[2]]))
    binaries = {}
    for name in names[:2]:
        data = fetch(selected[name])
        digest = hashlib.sha256(data).hexdigest()
        if expected.get(name) != digest:
            raise ValueError(f'SHA-256 mismatch or missing checksum: {name}')
        binaries[name] = {'url': selected[name], 'sha256': digest}
        if name.endswith('.zip'):
            with ZipFile(io.BytesIO(data)) as archive:
                members = archive.namelist()
                if 'Lumo/lumo.exe' not in members:
                    raise ValueError('Portable ZIP must contain Lumo/lumo.exe')
                for member in members:
                    path = PurePosixPath(member)
                    if path.is_absolute() or '..' in path.parts or '\\' in member or ':' in member:
                        raise ValueError('Unsafe portable ZIP path')
    return version, binaries[names[0]], binaries[names[1]]


def manifests(version, tag, setup, portable):
    homepage = f'https://github.com/{SOURCE}'
    common = {'PackageIdentifier': PACKAGE_ID, 'PackageVersion': version}
    installer = {**common, 'InstallerLocale': 'en-US', 'InstallerType': 'nullsoft',
                 'Scope': 'user', 'MinimumOSVersion': '10.0.0.0',
                 'UpgradeBehavior': 'install',
                 'InstallerSwitches': {'Silent': '/S', 'SilentWithProgress': '/S'},
                 'AppsAndFeaturesEntries': [{'DisplayName': 'Lumo', 'Publisher': 'mthlpbs',
                                            'DisplayVersion': version, 'ProductCode': 'Lumo'}],
                 'Installers': [{'Architecture': 'x64', 'InstallerUrl': setup['url'],
                                 'InstallerSha256': setup['sha256'].upper()}],
                 'ManifestType': 'installer', 'ManifestVersion': SCHEMA}
    locale = {**common, 'PackageLocale': 'en-US', 'Publisher': 'mthlpbs',
              'PublisherUrl': 'https://github.com/mthlpbs',
              'PublisherSupportUrl': homepage + '/issues', 'PackageName': 'Lumo',
              'PackageUrl': homepage, 'License': 'MIT',
              'LicenseUrl': homepage + '/blob/' + quote(tag, safe='') + '/LICENSE',
              'ShortDescription': 'A desktop island for music, focus and quick tools on Windows.',
              'Moniker': 'lumo', 'Tags': ['desktop', 'dynamic-island', 'music', 'productivity'],
              'ReleaseNotesUrl': homepage + '/releases/tag/' + quote(tag, safe=''),
              'ManifestType': 'defaultLocale', 'ManifestVersion': SCHEMA}
    winget = {PACKAGE_ID + '.yaml': {**common, 'DefaultLocale': 'en-US',
                                   'ManifestType': 'version', 'ManifestVersion': SCHEMA},
              PACKAGE_ID + '.installer.yaml': installer,
              PACKAGE_ID + '.locale.en-US.yaml': locale}
    scoop = {'version': version, 'description': locale['ShortDescription'],
             'homepage': homepage, 'license': 'MIT',
             'architecture': {'64bit': {'url': portable['url'], 'hash': portable['sha256']}},
             'extract_dir': 'Lumo', 'bin': 'lumo.exe', 'shortcuts': [['lumo.exe', 'Lumo']],
             'pre_install': "if (Get-Process lumo -ErrorAction SilentlyContinue) { throw 'Close Lumo before installing or updating it with Scoop.' }",
             'notes': ['Settings stay in %APPDATA%\\Lumo.',
                       'Use scoop update lumo for this installation; the built-in Setup updater installs a separate per-user copy.']}
    return winget, scoop


def yaml_lines(value, level=0):
    """Emit the small manifest subset without a runtime PyYAML dependency."""
    prefix = ' ' * level
    if isinstance(value, dict):
        for key, item in value.items():
            if isinstance(item, (dict, list)):
                yield f'{prefix}{key}:'
                yield from yaml_lines(item, level + 2)
            else:
                yield f'{prefix}{key}: {json.dumps(item, ensure_ascii=False)}'
    elif isinstance(value, list):
        for item in value:
            if isinstance(item, (dict, list)):
                nested = list(yaml_lines(item, level + 2))
                yield prefix + '- ' + nested[0].lstrip()
                yield from nested[1:]
            else:
                yield prefix + '- ' + json.dumps(item, ensure_ascii=False)


def generate(tag, output, token=''):
    release_version(tag)  # Reject non-app tags before any network request.
    release = github(f'repos/{SOURCE}/releases/tags/{quote(tag, safe="")}', token)
    version, setup, portable = release_assets(release, tag)
    winget, scoop = manifests(version, tag, setup, portable)
    directory = output / 'winget' / 'manifests/m/mthlpbs/Lumo' / version
    directory.mkdir(parents=True, exist_ok=True)
    for name, data in winget.items():
        schema_type = data['ManifestType']
        schema_url = f'https://aka.ms/winget-manifest.{schema_type}.{SCHEMA}.schema.json'
        (directory / name).write_text('# yaml-language-server: $schema=' + schema_url + '\n' +
                                      '\n'.join(yaml_lines(data)) + '\n', encoding='utf-8')
    (output / 'scoop').mkdir(parents=True, exist_ok=True)
    (output / 'scoop/lumo.json').write_text(json.dumps(scoop, indent=4) + '\n', encoding='utf-8')
    print(f'Generated {PACKAGE_ID} {version} manifests from verified published assets.')
    return version, directory


def publish_scoop(path, bucket, branch, token, api=github):
    repository(bucket)
    if not token:
        raise ValueError('SCOOP_BUCKET_TOKEN is required to publish')
    if bucket.lower() == SOURCE.lower():
        raise ValueError('Use a separate Scoop bucket repository, not Lumo source')
    repo = api(f'repos/{bucket}', token)
    if repo['private']:
        raise ValueError('The Scoop bucket must be public for users to add it')
    if repo['default_branch'] != branch:
        raise ValueError('SCOOP_BUCKET_BRANCH must be the bucket default branch used by scoop bucket add')
    manifest = json.loads(path.read_text(encoding='utf-8'))
    version = validate(manifest['version'])
    endpoint = f'repos/{bucket}/contents/bucket/lumo.json'
    # Retry optimistic SHA conflicts; re-read before writing so an older run
    # can never overwrite a newer manifest after a concurrent release.
    for attempt in range(3):
        previous = api(endpoint + '?' + urlencode({'ref': branch}), token, missing=True)
        if previous:
            current = json.loads(base64.b64decode(previous['content']))
            current_version = validate(current['version'])
            if current_version > version or current == manifest:
                print('Scoop already contains this version or a newer one; no commit needed.')
                return
            if current_version == version:
                raise ValueError('Scoop has different metadata for this version; review before replacing it')
        data = {'message': f'lumo: Update to version {manifest["version"]}', 'branch': branch,
                'content': base64.b64encode(path.read_bytes()).decode('ascii')}
        if previous:
            data['sha'] = previous['sha']
        try:
            result = api(endpoint, token, method='PUT', data=data)
            print('Scoop manifest updated: ' + result['content']['html_url'])
            return
        except ValueError as error:
            if not any(code in str(error) for code in ['HTTP 409:', 'HTTP 422:']) or attempt == 2:
                raise


def winget_needed(version, token, api=github):
    validate(version)
    path = f'repos/microsoft/winget-pkgs/contents/manifests/m/mthlpbs/Lumo/{version}'
    if api(path, token, missing=True) is not None:
        print('WinGet already contains this version; no submission needed.')
        return False
    title = f'New version: {PACKAGE_ID} version {version}'
    query = f'repo:microsoft/winget-pkgs is:pr is:open "{PACKAGE_ID}" "{version}" in:title'
    results = api('search/issues?' + urlencode({'q': query, 'per_page': 100}), token)
    if any(item['title'] == title for item in results['items']):
        print('A matching WinGet pull request is already open; no submission needed.')
        return False
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    build = commands.add_parser('generate')
    build.add_argument('--tag', required=True)
    build.add_argument('--output', type=Path, required=True)
    build.add_argument('--github-output', type=Path)
    scoop = commands.add_parser('scoop')
    scoop.add_argument('--manifest', type=Path, required=True)
    scoop.add_argument('--bucket', required=True)
    scoop.add_argument('--branch', default='main')
    winget = commands.add_parser('winget')
    winget.add_argument('--manifest-dir', type=Path, required=True)
    winget.add_argument('--version', required=True)
    winget.add_argument('--tool', type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == 'generate':
            version, directory = generate(args.tag, args.output, os.environ.get('GH_TOKEN', ''))
            if args.github_output:
                with args.github_output.open('a', encoding='utf-8') as out:
                    out.write(f'version={version}\nmanifest_dir={directory.as_posix()}\n')
        elif args.command == 'scoop':
            publish_scoop(args.manifest, args.bucket, args.branch, os.environ.get('SCOOP_BUCKET_TOKEN', ''))
        else:
            token = os.environ.get('WINGET_CREATE_GITHUB_TOKEN', '')
            if not token:
                raise ValueError('WINGET_CREATE_GITHUB_TOKEN is required to submit')
            if winget_needed(args.version, token):
                subprocess.run([str(args.tool.resolve()), 'submit', '--no-open', '--prtitle',
                                f'New version: {PACKAGE_ID} version {args.version}',
                                str(args.manifest_dir.resolve())], check=True)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()

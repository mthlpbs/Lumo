"""Verify the external protection add-on and stage it beside a local build.

No release is published. The manifest is the immutable input for the named
add-on release; obtain the listed snapshots before running this command.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, default=ROOT / ".tools/adblock-data")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads((ROOT / "addons/web-blocking/manifest.json").read_text())
    staged = args.output / "protection" / manifest["tag"]
    bodies = []
    for entry in manifest["files"]:
        data = (args.input / entry["name"]).read_bytes()
        if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
            raise SystemExit(f"Protection snapshot mismatch: {entry['name']}")
        bodies.append((entry["name"], data))
    staged.mkdir(parents=True, exist_ok=True)
    for name, data in bodies:
        (staged / name).write_bytes(data)
    shutil.copyfile(ROOT / "addons/web-blocking/manifest.json", staged / "manifest.json")
    shutil.copyfile(ROOT / "addons/web-blocking/README.md", staged / "README.md")
    print(f"Verified {len(bodies)} add-on files ({sum(len(b) for _, b in bodies):,} bytes): {staged}")


if __name__ == "__main__":
    main()

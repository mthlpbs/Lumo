"""Create the portable ZIP directly, without an unpacked staging folder."""

import argparse
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    files = [args.exe, root / "LICENSE", root / "THIRD-PARTY-NOTICES.txt"]
    for file in files:
        if not file.is_file():
            parser.error(f"Missing portable file: {file}")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with ZipFile(args.out, "w", compression=ZIP_DEFLATED, compresslevel=6) as archive:
        for file in files:
            name = "lumo.exe" if file == args.exe else file.name
            archive.write(file, arcname=f"Lumo/{name}")


if __name__ == "__main__":
    main()

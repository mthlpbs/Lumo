"""Verify portable ZIP structure, licenses and executable identity."""
import argparse
import hashlib
from pathlib import Path
import struct
from zipfile import ZipFile


def verify(exe, portable, setup, root):
    payload = exe.read_bytes()
    offset = struct.unpack_from('<I', payload, 0x3C)[0]
    if payload[offset:offset + 4] != b'PE\0\0' or struct.unpack_from('<H', payload, offset + 4)[0] != 0x8664:
        raise ValueError('Executable is not a Windows x64 app')
    with ZipFile(portable) as archive:
        if archive.testzip() is not None:
            raise ValueError('Portable archive integrity check failed')
        if sorted(archive.namelist()) != ['Lumo/LICENSE','Lumo/THIRD-PARTY-NOTICES.txt','Lumo/WebView2Loader.dll','Lumo/lumo.exe']:
            raise ValueError('Portable archive has unexpected files or paths')
        if hashlib.sha256(archive.read('Lumo/lumo.exe')).digest() != hashlib.sha256(payload).digest():
            raise ValueError('Portable executable differs from the built app')
        for name in ('LICENSE','THIRD-PARTY-NOTICES.txt'):
            if archive.read('Lumo/' + name) != (root / name).read_bytes():
                raise ValueError('Portable license differs from source: ' + name)
        loader = (root / 'crates/lumo/vendor/webview2/WebView2Loader.dll').read_bytes()
        if hashlib.sha256(loader).hexdigest() != '465a7ddfb3a0da4c3965daf2ad6ac7548513f42329b58aebc337311c10ea0a6f':
            raise ValueError('Microsoft WebView2 loader checksum does not match the pinned SDK')
        if archive.read('Lumo/WebView2Loader.dll') != loader:
            raise ValueError('Portable WebView2 loader differs from the verified Microsoft binary')
    if not setup.is_file() or setup.stat().st_size < 1024:
        raise ValueError('Setup file is missing or incomplete')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('exe','portable','setup'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    verify(args.exe,args.portable,args.setup,Path(__file__).resolve().parent.parent)
    print('Verified Windows x64 executable, portable contents and licenses; setup present.')

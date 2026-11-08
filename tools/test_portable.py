"""The portable package must contain the small locator, never a browser engine."""
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED
import verify_packages

ROOT = Path(__file__).resolve().parent.parent


class PortableTests(unittest.TestCase):
    def test_exact_payload_and_loader_integrity(self):
        with tempfile.TemporaryDirectory(prefix='lumo-portable-') as folder:
            folder = Path(folder)
            exe = folder / 'lumo.exe'
            payload = bytearray(512)
            struct.pack_into('<I', payload, 0x3C, 0x80)
            payload[0x80:0x84] = b'PE\0\0'
            struct.pack_into('<H', payload, 0x84, 0x8664)
            exe.write_bytes(payload)
            setup = folder / 'setup.exe'
            setup.write_bytes(b'fixture' * 512)
            archive = folder / 'portable.zip'
            subprocess.run([sys.executable, str(ROOT / 'tools/package_portable.py'),
                            '--exe', str(exe), '--out', str(archive)], check=True)
            verify_packages.verify(exe, archive, setup, ROOT)
            with ZipFile(archive) as source:
                files = {name: source.read(name) for name in source.namelist()}
            self.assertEqual(sorted(files), ['Lumo/LICENSE', 'Lumo/THIRD-PARTY-NOTICES.txt',
                                             'Lumo/WebView2Loader.dll', 'Lumo/lumo.exe'])
            files['Lumo/WebView2Loader.dll'] = b'tampered'
            with ZipFile(archive, 'w', compression=ZIP_DEFLATED) as target:
                for name, data in files.items():
                    target.writestr(name, data)
            with self.assertRaisesRegex(ValueError, 'loader differs'):
                verify_packages.verify(exe, archive, setup, ROOT)


if __name__ == '__main__':
    unittest.main()

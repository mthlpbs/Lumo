"""Version alignment, rollover, rebuild identity and failure-path checks."""
from datetime import date
from pathlib import Path
import tempfile
import unittest
import version


class VersionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='lumo-version-')
        self.root = Path(self.temporary.name)
        for name in version.FILES:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((version.ROOT / name).read_bytes())
        version.set_version(self.root, '2026.1107.2')

    def tearDown(self):
        self.temporary.cleanup()

    def test_date_rollover_and_build_counter(self):
        self.assertEqual(version.next_version('2026.1108.1', date(2026,11,8)), '2026.1108.2')
        self.assertEqual(version.next_version('2026.1107.20', date(2026,11,8)), '2026.1108.1')
        self.assertEqual(version.next_version('2026.1231.8', date(2027,1,1)), '2027.101.1')
        for current, today in [('2026.1108.65535',date(2026,11,8)), ('2026.1108.1',date(2026,11,7))]:
            with self.assertRaises(ValueError):
                version.next_version(current,today)

    def test_rejects_invalid_windows_and_date_versions(self):
        for value in ['2026.0108.1','2026.1108.0','2026.229.1','2026.1331.1','2026.1108.65536','65536.101.1','v2026.1108.1']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                version.validate(value)
        self.assertEqual(version.validate('2028.229.1'), (2028,229,1))

    def test_all_six_files_align_without_touching_other_dependencies(self):
        before = version.read_files(self.root)
        version.set_version(self.root,'2026.1108.1')
        self.assertEqual(version.check(self.root),'2026.1108.1')
        # Normalizing version fields must reproduce all other original bytes.
        self.assertEqual(version.render(before,'0.0.0'), version.render(version.read_files(self.root),'0.0.0'))

    def test_failure_prepares_no_partial_edits(self):
        path = self.root/'installer/lumo.nsi'
        path.write_bytes(path.read_bytes().replace(b'!define VERSION ',b'!define BROKEN '))
        before = version.read_files(self.root)
        with self.assertRaises(ValueError):
            version.set_version(self.root,'2026.1108.1')
        self.assertEqual(before,version.read_files(self.root))

    def test_unchanged_and_failed_rebuilds_keep_the_version(self):
        first = version.plan(self.root,date(2026,11,8))
        self.assertEqual(first['version'],'2026.1108.1')
        self.assertEqual(version.plan(self.root,date(2026,11,9))['version'],first['version'])
        version.verify(self.root,complete=True)
        self.assertEqual(version.plan(self.root,date(2026,11,10))['version'],first['version'])

    def test_source_change_advances_once_and_is_detected_during_build(self):
        version.plan(self.root,date(2026,11,8))
        source=self.root/'crates/lumo/src/new.rs'
        source.parent.mkdir(parents=True,exist_ok=True)
        source.write_text('// new behavior\n')
        with self.assertRaises(ValueError):
            version.verify(self.root)
        self.assertEqual(version.plan(self.root,date(2026,11,8))['version'],'2026.1108.2')
        self.assertEqual(version.plan(self.root,date(2026,11,8))['version'],'2026.1108.2')

    def test_explicit_and_forced_versions(self):
        self.assertEqual(version.plan(self.root,date(2026,11,8),version='2026.1108.5')['version'],'2026.1108.5')
        self.assertEqual(version.plan(self.root,date(2026,11,8),bump=True)['version'],'2026.1108.6')
        self.assertEqual(version.plan(self.root,date(2026,11,9),keep=True)['version'],'2026.1108.6')


if __name__ == '__main__':
    unittest.main()

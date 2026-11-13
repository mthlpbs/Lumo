"""Package-manager publication contracts; all writes and releases are mocked."""
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest
from zipfile import ZipFile

import package_managers as pm


def fixture(tag='2026.1109.1'):
    version = tag.removeprefix('v')
    setup = f'Lumo-Setup-{version}.exe'
    portable = f'Lumo-{version}-portable.zip'
    buf = io.BytesIO()
    with ZipFile(buf, 'w') as archive:
        archive.writestr('Lumo/lumo.exe', b'fixture')
        archive.writestr('Lumo/WebView2Loader.dll', b'locator fixture')
    payloads = {setup: b'NSIS fixture', portable: buf.getvalue()}
    payloads['SHA256SUMS.txt'] = ''.join(f'{hashlib.sha256(data).hexdigest()}  {name}\n'
                                       for name, data in payloads.items()).encode()
    assets = [{'name': name, 'browser_download_url': f'https://github.com/{pm.SOURCE}/releases/download/{tag}/{name}'}
              for name in payloads]
    release = {'tag_name': tag, 'draft': False, 'prerelease': False, 'assets': assets}
    return release, payloads, lambda url: payloads[url.rsplit('/', 1)[1]]


class ManifestTests(unittest.TestCase):
    def test_workflow_keeps_winget_paused_even_with_credentials_and_scoop_enabled(self):
        root = Path(__file__).resolve().parents[1]
        workflow = (root / '.github/workflows/package-managers.yml').read_text(encoding='utf-8')
        self.assertNotIn('\n  winget:\n', workflow)
        self.assertNotIn('WINGET_TOKEN', workflow)
        self.assertNotIn('WINGET_TOKEN', (root / '.github/workflows/ci.yml').read_text(encoding='utf-8'))
        self.assertIn("if: inputs.publish && needs.prepare.outputs.scoop == 'true'", workflow)
        step = workflow.split('      - name: Check package-manager configuration\n', 1)[1]
        script = step.split('        run: |\n', 1)[1].split('      - name:', 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            output, summary = (Path(directory) / name for name in ['output', 'summary'])
            result = subprocess.run([sys.executable, '-c', textwrap.dedent(script)], capture_output=True, text=True,
                                    env={**os.environ, 'WINGET_TOKEN': 'fixture', 'SCOOP_BUCKET_TOKEN': 'fixture',
                                         'SCOOP_BUCKET_REPO': 'test/bucket', 'GITHUB_OUTPUT': str(output),
                                         'GITHUB_STEP_SUMMARY': str(summary)})
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text().splitlines(), ['scoop=true'])
            self.assertIn('WinGet publishing: paused', summary.read_text())

    def test_first_release_manifests_have_exact_immutable_urls_and_payload_hashes(self):
        for tag in ['2026.1109.1', 'v2026.1109.1']:
            release, data, fetch = fixture(tag)
            version, setup, portable = pm.release_assets(release, tag, fetch)
            winget, scoop = pm.manifests(version, tag, setup, portable)
            installer = winget['mthlpbs.Lumo.installer.yaml']
            self.assertEqual(installer['InstallerType'], 'nullsoft')
            self.assertEqual(installer['Scope'], 'user')
            self.assertEqual(installer['AppsAndFeaturesEntries'][0]['ProductCode'], 'Lumo')
            self.assertEqual(installer['Installers'][0]['InstallerSha256'], hashlib.sha256(data[f'Lumo-Setup-{version}.exe']).hexdigest().upper())
            self.assertIn(f'/download/{tag}/', scoop['architecture']['64bit']['url'])
            self.assertEqual(scoop['extract_dir'], 'Lumo')
            self.assertEqual(scoop['architecture']['64bit']['hash'], portable['sha256'])
            self.assertNotIn('latest', json.dumps(winget) + json.dumps(scoop))

    def test_draft_prerelease_wrong_tag_and_missing_assets_cannot_publish(self):
        for field in ['draft', 'prerelease']:
            release, _, fetch = fixture()
            release[field] = True
            with self.assertRaises(ValueError):
                pm.release_assets(release, '2026.1109.1', fetch)
        release, _, fetch = fixture()
        with self.assertRaises(ValueError):
            pm.release_assets(release, '2026.1109.2', fetch)
        release['assets'].pop()
        with self.assertRaises(ValueError):
            pm.release_assets(release, '2026.1109.1', fetch)
        for tag in ['addon-places', '2026.1109.1-preview', '2026.0230.1', '../2026.1109.1']:
            with self.assertRaises(ValueError):
                pm.release_version(tag)

    def test_tampered_asset_checksum_duplicate_and_external_url_are_refused(self):
        release, data, fetch = fixture()
        data['Lumo-Setup-2026.1109.1.exe'] = b'tampered'
        with self.assertRaisesRegex(ValueError, 'mismatch'):
            pm.release_assets(release, '2026.1109.1', fetch)
        sums = b'1' * 64 + b'  file.zip\n'
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            pm.checksums(sums + sums)
        release, _, fetch = fixture()
        release['assets'][0]['browser_download_url'] = 'https://example.org/setup.exe'
        with self.assertRaisesRegex(ValueError, 'host/path'):
            pm.release_assets(release, '2026.1109.1', fetch)

    def test_unsafe_or_unusable_portable_is_refused_even_with_matching_checksum(self):
        for names in [['other.exe'], ['Lumo/lumo.exe', '../escape']]:
            release, data, fetch = fixture()
            buf = io.BytesIO()
            with ZipFile(buf, 'w') as archive:
                for name in names:
                    archive.writestr(name, b'fixture')
            data['Lumo-2026.1109.1-portable.zip'] = buf.getvalue()
            data['SHA256SUMS.txt'] = ''.join(f'{hashlib.sha256(value).hexdigest()}  {key}\n'
                for key, value in data.items() if key != 'SHA256SUMS.txt').encode()
            with self.assertRaises(ValueError):
                pm.release_assets(release, '2026.1109.1', fetch)

    def test_scoop_bootstrap_retry_no_downgrade_and_conflicting_version(self):
        release, _, fetch = fixture()
        version, setup, portable = pm.release_assets(release, '2026.1109.1', fetch)
        _, candidate = pm.manifests(version, '2026.1109.1', setup, portable)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'lumo.json'
            path.write_text(json.dumps(candidate), encoding='utf-8')
            for current_version in [None, '2026.1109.1', '2026.1110.1', '2026.1108.1']:
                writes = []
                def api(endpoint, token, method='GET', data=None, **_):
                    if method == 'PUT':
                        writes.append(data)
                        return {'content': {'html_url': 'https://github.com/test/bucket/blob/main/bucket/lumo.json'}}
                    if '/contents/' not in endpoint:
                        return {'private': False, 'default_branch': 'main'}
                    if current_version is None:
                        return None
                    current = {**candidate, 'version': current_version}
                    return {'sha': 'previous', 'content': base64.b64encode(json.dumps(current).encode()).decode()}
                pm.publish_scoop(path, 'test/bucket', 'main', 'fixture-token', api)
                self.assertEqual(len(writes), int(current_version in [None, '2026.1108.1']))
                if writes:
                    self.assertEqual(json.loads(base64.b64decode(writes[0]['content'])), candidate)
            def conflict(endpoint, token, method='GET', **_):
                if '/contents/' not in endpoint:
                    return {'private': False, 'default_branch': 'main'}
                current = {**candidate, 'description': 'different manifest'}
                return {'sha': 'old', 'content': base64.b64encode(json.dumps(current).encode()).decode()}
            with self.assertRaisesRegex(ValueError, 'different metadata'):
                pm.publish_scoop(path, 'test/bucket', 'main', 'fixture', conflict)

    def test_winget_duplicate_pr_or_merged_version_is_not_submitted_twice(self):
        self.assertFalse(pm.winget_needed('2026.1109.1', 'fixture', lambda *a, **k: []))
        def api(endpoint, *_args, **_kwargs):
            if endpoint.startswith('repos/'):
                return None
            return {'items': [{'title': 'New version: mthlpbs.Lumo version 2026.1109.1'}]}
        self.assertFalse(pm.winget_needed('2026.1109.1', 'fixture', api))
        self.assertTrue(pm.winget_needed('2026.1109.2', 'fixture', api))

    def test_scoop_rereads_after_conflict_and_does_not_overwrite_newer_release(self):
        release, _, fetch = fixture()
        version, setup, portable = pm.release_assets(release, '2026.1109.1', fetch)
        _, candidate = pm.manifests(version, '2026.1109.1', setup, portable)
        for status in [409, 422]:
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'lumo.json'
                path.write_text(json.dumps(candidate), encoding='utf-8')
                calls = {'reads': 0, 'writes': 0}
                def api(endpoint, token, method='GET', **_):
                    if method == 'PUT':
                        calls['writes'] += 1
                        raise ValueError(f'GitHub HTTP {status}: concurrent update')
                    if '/contents/' not in endpoint:
                        return {'private': False, 'default_branch': 'main'}
                    calls['reads'] += 1
                    if calls['reads'] == 1:
                        return None
                    newer = {**candidate, 'version': '2026.1110.1'}
                    return {'sha': 'newer', 'content': base64.b64encode(json.dumps(newer).encode()).decode()}
                pm.publish_scoop(path, 'test/bucket', 'main', 'fixture', api)
                self.assertEqual(calls, {'reads': 2, 'writes': 1})

    def test_scoop_refuses_private_bucket_or_invisible_nondefault_branch(self):
        for metadata in [{'private': True, 'default_branch': 'main'},
                         {'private': False, 'default_branch': 'master'}]:
            with self.assertRaises(ValueError):
                pm.publish_scoop(Path('unused'), 'test/bucket', 'main', 'fixture',
                                 lambda *args, **kwargs: metadata)


if __name__ == '__main__':
    unittest.main()

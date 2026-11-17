import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import hashlib

spec = importlib.util.spec_from_file_location('forge_package', Path(__file__).parents[1] / 'package.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
verify_spec = importlib.util.spec_from_file_location('verify_release', Path(__file__).parents[1] / 'verify_release.py')
verifier = importlib.util.module_from_spec(verify_spec)
verify_spec.loader.exec_module(verifier)


class WebsiteManifestTests(unittest.TestCase):
    def test_checksum_file_must_match_every_download_exactly_once(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            name = 'Forge-test.zip'
            (folder / name).write_bytes(b'package')
            digest = hashlib.sha256(b'package').hexdigest()
            record = {'file': name, 'bytes': 7, 'sha256': digest}
            (folder / 'test.artifact.json').write_text(json.dumps({'version': '1.0.0', 'downloads': [record]}))
            package.manifest(folder, '1.0.0')
            verifier.verify(folder)
            correct = f'{digest}  {name}\n'
            for checksums in ('', f'{"0" * 64}  {name}\n', correct * 2,
                              correct + f'{digest}  untracked.zip\n'):
                with self.subTest(checksums=checksums):
                    (folder / 'SHA256SUMS').write_text(checksums)
                    with self.assertRaisesRegex(ValueError, 'checksum'):
                        verifier.verify(folder)

    def test_feed_hashes_are_checked_even_when_website_metadata_was_replaced(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            name = 'Forge-1.0.1-win-x64-full.nupkg'
            (folder / name).write_bytes(b'tampered')
            item = {'file': name, 'bytes': 8, 'sha256': hashlib.sha256(b'tampered').hexdigest()}
            (folder / 'downloads.json').write_text(json.dumps({'downloads': [item]}))
            asset = {'FileName': name, 'Size': 8, 'SHA256': hashlib.sha256(b'original').hexdigest()}
            (folder / 'releases.win-x64.json').write_text(json.dumps({'Assets': [asset]}))
            with self.assertRaisesRegex(ValueError, 'Invalid update package'):
                verifier.verify(folder)

    def test_merged_downloads_verify_actual_artifact_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            artifact = folder / 'Forge-1.0.0-linux-x64.deb'
            artifact.write_bytes(b'package')
            record = {'os': 'linux', 'arch': 'x64', 'file': artifact.name, 'bytes': 7,
                      'sha256': 'bc4a71180870f7945155fbb02f4b0a2e3faa2a62d6d31b7039013055ed19869a'}
            # Independently known SHA-256 for b'package'.
            (folder / 'linux.artifact.json').write_text(json.dumps({'version': '1.0.0', 'downloads': [record]}))
            package.manifest(folder, '1.0.0')
            self.assertEqual(json.loads((folder / 'downloads.json').read_text())['downloads'][0]['file'], artifact.name)
            artifact.write_bytes(b'tampered')
            with self.assertRaises(ValueError):
                package.manifest(folder, '1.0.0')

    def test_empty_directory_cannot_be_presented_as_a_release(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ValueError):
                package.manifest(Path(temp), '1.0.0')

    def test_mixed_versions_and_paths_outside_downloads_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            part = folder / 'bad.artifact.json'
            part.write_text(json.dumps({'version': '0.9.0', 'downloads': []}))
            with self.assertRaisesRegex(ValueError, 'Mixed release versions'):
                package.manifest(folder, '1.0.0')
            for filename in ('../private', '/private', '..\\private'):
                part.write_text(json.dumps({'version': '1.0.0', 'downloads': [{'file': filename}]}))
                with self.assertRaisesRegex(ValueError, 'plain filename'):
                    package.manifest(folder, '1.0.0')


if __name__ == '__main__':
    unittest.main()

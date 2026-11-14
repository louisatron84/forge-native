"""Verify a staged website release, including every Velopack package hash in each feed."""
import argparse
import hashlib
import json
from pathlib import Path


def verify(folder):
    manifest = json.loads((folder / 'downloads.json').read_text())
    names = set()
    for item in manifest['downloads']:
        name = item['file']
        if Path(name).name != name or '/' in name or '\\' in name or name in names:
            raise ValueError(f'Invalid or duplicate artifact name: {name}')
        names.add(name)
        path = folder / name
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        if digest != item['sha256'] or path.stat().st_size != item['bytes']:
            raise ValueError(f'Artifact does not match website metadata: {name}')
    if not names:
        raise ValueError('No downloadable artifacts')
    for feed in folder.glob('releases.*.json'):
        for asset in json.loads(feed.read_text())['Assets']:
            if asset['FileName'] not in names:
                raise ValueError(f'Update feed references an untracked package: {asset["FileName"]}')
            path = folder / asset['FileName']
            with path.open('rb') as stream:
                digest = hashlib.file_digest(stream, 'sha256').hexdigest()
            if digest.lower() != asset['SHA256'].lower() or path.stat().st_size != asset['Size']:
                raise ValueError(f'Invalid update package: {path.name}')
    print(f'Verified {len(names)} downloads and all update feeds')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('folder', type=Path)
    verify(parser.parse_args().folder)

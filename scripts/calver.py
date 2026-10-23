#!/usr/bin/env python3
"""Release versions: YYYY.M.N, with N starting at 0 each UTC month.

This intentionally mirrors wslim's tag-based CalVer policy. It uses only
Python's standard library and updates Cargo.toml/Cargo.lock together.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
from pathlib import Path
import re
import subprocess
import sys
import tomllib

VALID = re.compile(r"^v(?P<year>\d{4})\.(?P<month>[1-9]|1[0-2])\.(?P<revision>0|[1-9]\d*)$")
ROOT = Path(__file__).resolve().parent.parent


def next_version(tags: list[str], when: datetime) -> str:
    """Choose the highest existing revision for this UTC month plus one."""
    revisions = []
    for tag in tags:
        matched = VALID.fullmatch(tag.strip())
        if matched and int(matched['year']) == when.year and int(matched['month']) == when.month:
            revisions.append(int(matched['revision']))
    return f"{when.year}.{when.month}.{max(revisions, default=-1) + 1}"


def run(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def tags() -> list[str]:
    return run('git', 'tag', '--list', 'v*').splitlines()


def package_version(path: Path, package: str) -> str:
    obj = tomllib.loads(path.read_text(encoding='utf-8'))
    if path.name == 'Cargo.toml':
        return obj['package']['version']
    matching = [item['version'] for item in obj['package'] if item['name'] == package]
    if len(matching) != 1:
        raise ValueError(f'Expected exactly one {package} package in Cargo.lock')
    return matching[0]


def bump(version: str) -> None:
    path = ROOT / 'Cargo.toml'
    original = path.read_text(encoding='utf-8')
    pattern = re.compile(r'(?ms)^(\[package\]\s*.*?)(?=^\[|\Z)')
    found = pattern.search(original)
    if not found:
        raise ValueError('Missing [package] in Cargo.toml')
    block, count = re.subn(r'(?m)^version\s*=\s*"[^"]+"', f'version = "{version}"', found.group(1))
    if count != 1:
        raise ValueError('Expected exactly one package version')
    path.write_text(original[:found.start(1)] + block + original[found.end(1):], encoding='utf-8')
    subprocess.run(['cargo', 'generate-lockfile'], cwd=ROOT, check=True)
    assert_version(version)


def assert_version(version: str) -> None:
    if not VALID.fullmatch(f'v{version}'):
        raise ValueError(f'Invalid CalVer version: {version}')
    for filename in ('Cargo.toml', 'Cargo.lock'):
        current = package_version(ROOT / filename, 'forksync')
        if current != version:
            raise ValueError(f'{filename}: expected {version}, found {current}')


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('next', 'bump', 'validate-tag'))
    parser.add_argument('tag', nargs='?', help='Required by validate-tag')
    args = parser.parse_args()
    if args.action == 'validate-tag':
        if not args.tag or not VALID.fullmatch(args.tag):
            parser.error('Tag must match vYYYY.M.N (N starts at zero)')
        assert_version(args.tag[1:])
        print(f'Validated {args.tag}')
        return 0
    version = next_version(tags(), datetime.now(timezone.utc))
    if args.action == 'bump':
        if f'v{version}' in tags():
            raise ValueError(f'Release tag already exists: v{version}')
        bump(version)
    print(f'v{version}')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, subprocess.CalledProcessError) as exc:
        print(f'calver: {exc}', file=sys.stderr)
        sys.exit(1)

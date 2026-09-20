#!/usr/bin/env -S uv run
"""Regenerate the README screenshots from the deterministic preview fixtures.

The shots are plain preview captures without the preview banner, so they show
the installer's own layout; no disk, pool or package is touched. Build the
headless preview binary first, or run `just readme-shots`:

  SLINT_EMIT_DEBUG_INFO=1 cargo build -p archinstall-zfs-slint \
    --no-default-features --features desktop-mock,slint/mcp --locked
  uv run slint-ui/scripts/readme_shots.py
"""
import argparse
import os
from pathlib import Path
import shutil
import sys

from review import Preview

# README image stem -> preview scene, in the order the README shows them.
SHOTS = {
    'welcome-screen': 'welcome',
    'disk-step': 'disk',
    'zfs-step': 'zfs',
    'system-step': 'system',
    'users-step': 'users',
    'desktop-step': 'desktop',
    'review-step': 'review',
    'install-progress': 'install',
    'install-complete': 'done',
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/azfs'))
    parser.add_argument('--assets', type=Path, default=Path('assets'))
    parser.add_argument('--work', type=Path, default=Path('target/readme-shots'),
                        help='Screenshots, element trees and preview logs before copying')
    parser.add_argument('--size', default='1920x1080', help='The README baseline is Full HD')
    parser.add_argument('--scale', default='1')
    parser.add_argument('--shots', nargs='+', choices=list(SHOTS), default=list(SHOTS))
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.exists():
        sys.exit(f'{binary} is missing: build desktop-mock,slint/mcp first (see --help)')
    args.work.mkdir(parents=True, exist_ok=True)
    args.assets.mkdir(parents=True, exist_ok=True)
    # Inherited by every preview process this script starts.
    os.environ['AZFS_PREVIEW_BANNER'] = '0'
    failures = []
    for name in args.shots:
        preview = Preview(binary, SHOTS[name], args.size, args.scale, args.work)
        try:
            preview.ready()
            preview.screenshot(name)
        except Exception as error:  # noqa: BLE001 - report every scene, then exit non-zero
            failures.append(f'{name}: {type(error).__name__}: {error}')
            continue
        finally:
            preview.close()
        issues = preview.log_issues()
        if issues:
            failures.append(f'{name}: {len(issues)} log issue(s): {issues[0].strip()}')
            continue
        shutil.copyfile(args.work / f'{name}.png', args.assets / f'{name}.png')
        print(f'{args.assets / name}.png ({SHOTS[name]} @ {args.size}@{args.scale}x)', flush=True)
    for failure in failures:
        print(f'FAIL {failure}', file=sys.stderr)
    if failures:
        sys.exit(f'{len(failures)} of {len(args.shots)} screenshots failed; see {args.work}')
    print('Open the images and check them; a successful capture is not a review.')


if __name__ == '__main__':
    main()

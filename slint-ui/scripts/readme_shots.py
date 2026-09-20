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
import time

from review import Preview


def add_user(p):
    """The wizard keeps accounts, packages and services behind dialogs, so a
    page on its own never shows one. Fill the account form through the real
    controls, the way interactions.py does, and leave it open for the shot."""
    p.click('Button', 'User accounts')
    p.click('Button', 'Add another user')
    p.fill(0, 'maria')
    p.fill(1, 'a long preview passphrase for the README')
    p.fill(2, 'a long preview passphrase for the README')
    # The strength meter and the button state follow the input by a frame.
    time.sleep(.5)


# README image stem -> preview scene and the interaction to run before the
# capture, in the order the README shows them.
SHOTS = {
    'welcome-screen': ('welcome', None),
    'disk-step': ('disk', None),
    'zfs-step': ('zfs', None),
    'system-step': ('system', None),
    'users-step': ('users', add_user),
    'desktop-step': ('desktop', None),
    'review-step': ('review', None),
    'install-progress': ('install', None),
    'install-complete': ('done', None),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/azfs'))
    parser.add_argument('--assets', type=Path, default=Path('assets'))
    parser.add_argument('--work', type=Path, default=Path('target/readme-shots'),
                        help='Screenshots, element trees and preview logs before copying')
    # 3:2 at 150%: the logical 1280x853 viewport is about as wide as the content
    # itself, so a shot has no empty bands beside the panel, and every element is
    # rendered at one and a half times the pixels a README scales it down from.
    parser.add_argument('--size', default='1920x1280')
    parser.add_argument('--scale', default='1.5')
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
        scene, action = SHOTS[name]
        preview = Preview(binary, scene, args.size, args.scale, args.work)
        try:
            preview.ready()
            if action:
                action(preview)
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
        print(f'{args.assets / name}.png ({scene} @ {args.size}@{args.scale}x)', flush=True)
    for failure in failures:
        print(f'FAIL {failure}', file=sys.stderr)
    if failures:
        sys.exit(f'{len(failures)} of {len(args.shots)} screenshots failed; see {args.work}')
    print('Open the images and check them; a successful capture is not a review.')


if __name__ == '__main__':
    main()

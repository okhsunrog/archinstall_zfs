#!/usr/bin/env -S uv run
"""Additional design-review scenarios for editor decisions, empty states and errors.

Uses the real Slint callbacks with desktop-mock fixtures. No installation is run.
"""
import argparse
from pathlib import Path

from review import Preview


def click_setting(p, name):
    p.reveal_by_tab('Button', name)
    p.click('Button', name)


def choice(p, label):
    for _ in range(100):
        e = p.element('ListItem', label)
        groups = [e for e in p.tree()['elements'] if e.get('accessibleRole') == 'Groupbox']
        panel = groups[-1]
        if e and e['absolutePosition']['y'] >= panel['absolutePosition']['y'] + 55 and e['absolutePosition']['y'] + e['size']['height'] <= panel['absolutePosition']['y'] + panel['size']['height'] - 60:
            p.click('ListItem', label)
            return
        p.key('\uf701')
    raise AssertionError(f'Could not reach choice {label}')


def accounts(p):
    p.click('Button', 'User accounts')
    p.click('Checkbox', 'Administrator alex')
    assert not p.element('Checkbox', 'Administrator alex').get('accessibleChecked', False)
    p.click('Checkbox', 'Administrator alex')
    assert p.element('Checkbox', 'Administrator alex').get('accessibleChecked', False)
    p.fill(0, 'Bad Name')
    p.fill(1, 'preview passphrase only')
    p.click('Button', 'Add user')
    p.wait('Text', 'Use up to 32')
    assert p.element('TextInput', 'Username').get('accessibleValue') == 'Bad Name'
    p.screenshot('invalid-username-preserved')
    p.fill(0, 'alex')
    p.click('Button', 'Add user')
    p.wait('Text', 'This username is already')
    p.screenshot('duplicate-username')
    p.fill(0, 'newuser')
    p.click('Button', 'Done')
    p.wait('Text', 'Click Add user')
    p.click('Button', 'Add user')
    p.wait('Button', 'Remove user newuser')
    p.screenshot('account-added')
    # Reopen to inspect and edit the saved account list at the top of the scroll view.
    p.click('Button', 'Done')
    p.click('Button', 'User accounts')
    p.click('Button', 'Remove user alex')
    p.click('Button', 'Remove user newuser')
    p.wait('Text', 'No user accounts yet')
    p.screenshot('empty-account-list')
    p.key('\u001b')
    assert p.element('Groupbox', 'User accounts') is None


def system(p):
    click_setting(p, 'Distribution')
    p.screenshot('distributions')
    p.click('Button', 'Cancel')
    click_setting(p, 'Timezone')
    p.screenshot('timezone-region')
    choice(p, 'Europe')
    p.fill(0, 'Mosc')
    p.wait('ListItem', 'Moscow')
    p.screenshot('timezone-city')
    choice(p, 'Moscow')
    p.wait('Text', 'Europe/Moscow')
    click_setting(p, 'Keyboard layout')
    p.fill(0, 'ru')
    p.screenshot('keyboard-filter')
    p.key('\u001b')
    assert p.element('Groupbox', 'Keyboard layout') is None
    click_setting(p, 'Locale')
    p.fill(0, 'does-not-exist-zzzzz')
    p.wait('Text', 'No matching choices')
    p.screenshot('empty-locale-search')
    p.key('\u001b')
    assert p.element('Groupbox', 'Locale') is None
    click_setting(p, 'Parallel downloads')
    p.screenshot('download-concurrency')
    p.key('\u001b')
    p.screenshot('system')


def desktop(p):
    click_setting(p, 'Optional packages')
    p.click('Checkbox', 'flatpak')
    assert p.element('Checkbox', 'flatpak').get('accessibleChecked', False)
    p.screenshot('optional-package-selected')
    p.click('Button', 'Done')
    click_setting(p, 'Display manager')
    p.screenshot('display-managers')
    p.click('Button', 'Cancel')
    click_setting(p, 'GPU driver')
    p.screenshot('gpu-drivers')
    p.click('Button', 'Cancel')
    click_setting(p, 'Profile')
    choice(p, 'Sway')
    p.screenshot('wayland-profile')
    p.reveal_by_tab('Combobox', 'Seat access')
    p.click('Combobox', 'Seat access')
    p.screenshot('seat-access-choices')
    p.key('\u001b')
    click_setting(p, 'Profile')
    choice(p, 'Console only')
    assert p.element('Button', 'GPU driver') is None
    assert p.element('Button', 'Display manager') is None
    p.screenshot('console-profile')
    click_setting(p, 'Extra services')
    p.click('Button', 'Remove service sshd')
    p.wait('Text', 'No extra services selected')
    p.screenshot('empty-services')
    p.key('\u001b')
    click_setting(p, 'Extra packages')
    p.fill(0, 'no-such-package-zzzzz')
    p.wait('Text', 'No matching packages')
    p.screenshot('empty-package-search')
    p.fill(0, 'visual')
    p.click('Button', 'Search AUR')
    p.wait('Button', 'Add package visual-studio-code-bin')
    p.screenshot('aur-search')
    p.click('Button', 'Add package visual-studio-code-bin')
    # Remove the first entries to bring the newly added package into view.
    for name in ['firefox', 'git', 'htop', 'visual-studio-code-bin']:
        p.click('Button', 'Remove package ' + name)
    p.wait('Text', 'No extra packages selected')
    p.screenshot('empty-selected-packages')
    p.fill(0, 'fire')
    p.click('Button', 'Add package firefox')
    p.wait('Button', 'Remove package firefox')
    assert p.element('TextInput', 'Search packages').get('accessibleValue', '') == ''
    p.screenshot('package-added')
    p.click('Button', 'Done')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sizes', nargs='+', default=['1920x1080@1', '1366x768@1', '1280x800@1'])
    parser.add_argument('--flows', nargs='+', choices=['accounts', 'system', 'desktop'], default=['accounts', 'system', 'desktop'])
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    for spec in args.sizes:
        size, scale = spec.split('@')
        for flow in args.flows:
            output = args.output / spec / flow
            output.mkdir(parents=True, exist_ok=True)
            p = Preview(Path('target/debug/azfs').resolve(), 'users' if flow == 'accounts' else flow, size, scale, output)
            try:
                p.ready()
                globals()[flow](p)
                print(f'PASS {spec} {flow}', flush=True)
            except Exception:
                p.screenshot('failure')
                raise
            finally:
                p.close()


if __name__ == '__main__':
    main()

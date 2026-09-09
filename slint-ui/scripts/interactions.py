#!/usr/bin/env -S uv run
"""Exercise actual UI callbacks on preview fixtures and save review screenshots."""
import argparse
from pathlib import Path
import time

from review import Preview


def system(p):
    p.click('Button', 'Hostname')
    p.fill(0, 'arch-workstation-long-name')
    p.screenshot('hostname')
    p.click('Button', 'OK')
    p.click('Button', 'Kernel')
    p.screenshot('kernel')
    p.click('ListItem', 'linux — compatible')
    p.click('Button', 'Locale')
    p.fill(0, 'ru_RU')
    p.screenshot('locale-filter')
    p.click('ListItem', 'ru_RU.UTF-8')
    p.key('5')
    p.wait('Text', '4 / 6')


def users(p):
    p.click('Button', 'Root password')
    p.fill(0, 'short')
    time.sleep(.5)
    p.screenshot('root-password-weak')
    p.fill(0, 'a long preview passphrase for testing')
    p.fill(0, '')
    time.sleep(.5)
    assert p.element('Text', 'Very weak') is None
    assert p.element('Text', 'Very strong') is None
    p.screenshot('root-password-cleared')
    p.click('Button', 'Cancel')
    p.click('Button', 'User accounts')
    p.screenshot('users-dialog')
    p.fill(0, 'previewuser')
    p.fill(1, 'a long preview passphrase for testing')
    time.sleep(.5)
    p.screenshot('users-filled')
    p.click('Button', 'Add user')
    p.wait('Text', 'previewuser')
    p.screenshot('users-added')
    p.click('Button', 'Done')


def desktop(p):
    p.click('Button', 'Profile')
    p.screenshot('profile')
    p.click('ListItem', 'KDE Plasma')
    p.click('Button', 'Optional packages')
    p.screenshot('optional-packages')
    p.click('Button', 'Done')
    for _ in range(9):
        p.key('j')
    p.screenshot('desktop-scrolled')
    p.click('Button', 'Extra packages')
    p.fill(0, 'fire')
    p.wait('Text', 'firefox')
    p.screenshot('packages')
    p.click('Button', 'Done')
    p.click('Button', 'Extra services')
    p.fill(0, 'cups')
    p.click('Button', 'Add')
    p.wait('Text', 'cups')
    p.screenshot('services-added')
    p.click('Button', 'Done')


def wifi(p):
    p.click('Button', 'Network settings')
    p.wait('ListItem', 'HomeNetwork')
    p.screenshot('wifi-picking')
    p.click('Button', 'Forget HomeNetwork')
    time.sleep(2)
    p.click('ListItem', 'HomeNetwork')
    p.wait('Button', 'Connect')  # forgotten network must ask for a password
    p.screenshot('wifi-forgotten-auth')
    p.click('Button', 'Cancel')
    p.click('Button', 'Rescan')
    p.wait('ListItem', 'Neighbour 5G')
    p.click('ListItem', 'Neighbour 5G')
    p.fill(0, 'preview-only-passphrase')
    p.screenshot('wifi-password')
    p.click('Button', 'Connect')
    p.screenshot('wifi-connecting')
    p.wait('Button', 'Other networks')
    assert p.element('Button', 'Disconnect') is None
    p.screenshot('wifi-connected')
    p.click('Button', 'Done')
    assert p.element('Groupbox', 'Network management') is None
    p.click('Button', 'Network settings')
    p.wait('Button', 'Disconnect')
    p.screenshot('wifi-management')
    p.click('Button', 'Disconnect')
    p.wait('ListItem', 'CorpNet')
    p.click('ListItem', 'CorpNet')
    p.wait('Button', 'Back')
    p.screenshot('wifi-enterprise-error')
    p.click('Button', 'Back')
    p.click('ListItem', 'Neighbour 5G')  # saved profile must skip auth
    p.wait('Button', 'Other networks')
    p.key('\n')
    assert p.element('Groupbox', 'Network management') is None


def inspect(p):
    p.click('Button', 'Import read-only')
    p.wait('Text', 'read-only demo import')
    p.screenshot('pool-imported')
    p.click('Button', 'Export')
    p.wait('Button', 'Import read-only')
    p.click('Button', 'Use')
    assert p.element('Button', 'Refresh') is None


def install(p):
    p.click('Button', 'Install')
    p.wait('Button', 'Cancel installation')
    installation_layout(p)
    p.screenshot('installation-started')
    p.wait('Button', 'Reboot')
    installation_layout(p)
    p.screenshot('installation-complete')


def cancel(p):
    p.click('Button', 'Cancel installation')
    p.wait('Text', 'Installation cancelled')
    installation_layout(p)
    p.screenshot('installation-cancelled')


def shell(p):
    p.wait('Button', 'Quit')
    p.wait('Button', 'Reboot')
    installation_layout(p)
    p.screenshot('completion')
    p.click('Button', 'Open installed system shell')
    p.wait('Text', 'Preview: shell closed')
    assert p.element('Button', 'Reboot') is not None
    assert p.element('Button', 'Quit') is not None
    installation_layout(p)
    p.screenshot('shell-returned')


def logs(p):
    def element_id(name):
        return next(e for e in p.tree()['elements'] if any(t.get('id') == name
                    for t in e.get('typeNamesAndIds', [])))
    log = element_id('InstallView::log-view')
    last = p.wait('Text', '[INFO] Installation complete!')
    assert last['absolutePosition']['y'] >= log['absolutePosition']['y']
    assert last['absolutePosition']['y'] + last['size']['height'] <= log['absolutePosition']['y'] + log['size']['height']
    thumb = element_id('ScrollBar::thumb')
    p.data('drag_element', elementHandle=thumb['handle'], target={
        'x': thumb['absolutePosition']['x'] + thumb['size']['width'] / 2,
        'y': log['absolutePosition']['y'] + 16,
    })
    p.wait('Button', 'Latest output')
    time.sleep(.3)
    installation_layout(p)
    p.screenshot('reading-earlier-output')
    p.click('Button', 'Latest output')
    assert p.element('Button', 'Latest output') is None
    last = p.wait('Text', '[INFO] Installation complete!')
    assert last['absolutePosition']['y'] >= log['absolutePosition']['y']
    p.screenshot('following-latest-output')


def installation_layout(p):
    tree = p.tree()['elements']
    def panel(name):
        return next(e for e in tree if any(t.get('id') == f'InstallView::{name}'
                    for t in e.get('typeNamesAndIds', [])))
    log = panel('log-panel')
    actions = panel('action-panel')
    y = actions['absolutePosition']['y']
    assert log['absolutePosition']['y'] + log['size']['height'] <= y - 12
    assert log['size']['height'] >= 100
    assert abs(y + actions['size']['height'] - (p.size[1] / p.scale - 20)) < 2
    buttons = [e for e in tree if e.get('accessibleRole') == 'Button'
               and e.get('accessibleLabel') != 'Latest output']
    for button in buttons:
        assert button['size']['height'] >= 44
        assert button['absolutePosition']['y'] >= y
        assert button['absolutePosition']['y'] + button['size']['height'] <= y + actions['size']['height']


def invalid(p):
    p.click('Button', 'Install')
    p.wait('Text', 'Validation:')
    assert p.element('Button', 'Cancel installation') is None
    p.screenshot('validation-blocks-installation')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/azfs'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sizes', nargs='+', default=['800x600@1', '1920x1080@2'])
    flows = ['system', 'users', 'desktop', 'wifi', 'inspect', 'install', 'cancel', 'shell', 'logs', 'invalid']
    parser.add_argument('--flows', nargs='+', choices=flows, default=flows)
    args = parser.parse_args()
    for spec in args.sizes:
        size, scale = spec.split('@')
        for flow in args.flows:
            output = args.output / spec / flow
            output.mkdir(parents=True, exist_ok=True)
            scene = {'wifi': 'offline', 'install': 'review', 'cancel': 'install', 'shell': 'done', 'logs': 'done'}.get(flow, flow)
            p = Preview(args.binary.resolve(), scene, size, scale, output)
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

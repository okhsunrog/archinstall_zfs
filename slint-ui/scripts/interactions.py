#!/usr/bin/env -S uv run
"""Exercise actual UI callbacks on preview fixtures and save review screenshots."""
import argparse
from pathlib import Path
import time

import invariants
from review import Preview, Results, SIZES


def system(p):
    p.click('Button', 'Hostname')
    p.fill(0, 'arch-workstation-long-name')
    p.screenshot('hostname')
    p.click('Button', 'Save')
    p.click('Button', 'Kernel')
    p.screenshot('kernel')
    p.click('ListItem', 'linux — compatible')
    p.reveal_by_tab('Button', 'Locale')
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
    # With an account present the dialog shows cards; the form is on request.
    p.click('Button', 'Add another user')
    p.fill(0, 'previewuser')
    p.fill(1, 'a long preview passphrase for testing')
    p.fill(2, 'a long preview passphrase for testing')
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
    p.reveal_by_tab('Button', 'Extra packages')
    p.screenshot('desktop-scrolled')
    p.click('Button', 'Extra packages')
    p.fill(0, 'fire')
    p.wait('Text', 'firefox')
    p.screenshot('packages')
    p.click('Button', 'Done')
    p.reveal_by_tab('Button', 'Extra services')
    p.click('Button', 'Extra services')
    p.fill(0, 'cups')
    p.click('Button', 'Add')
    p.wait('Text', 'cups')
    p.screenshot('services-added')
    p.click('Button', 'Done')


def wifi(p):
    p.screenshot('welcome-offline')
    p.click('Button', 'Wi-Fi & network')
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
    p.screenshot('welcome-connected')
    p.click('Button', 'Wi-Fi & network')
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
    thumb = next((e for e in p.tree()['elements'] if any(t.get('id') == 'ScrollBar::thumb'
                  for t in e.get('typeNamesAndIds', []))), None)
    if thumb is None:
        # At Full HD the entire fixture can fit without scrolling.
        first = p.wait('Text', '[INFO] Preview mode:')
        assert first['absolutePosition']['y'] >= log['absolutePosition']['y']
        assert p.element('Button', 'Latest output') is None
        installation_layout(p)
        p.screenshot('all-output-visible')
        return
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


def config_file(p):
    """Save the configuration to a file, then read it back."""
    import json
    import tempfile

    directory = Path(tempfile.mkdtemp())
    saved = directory / 'azfs-config.json'
    p.wait('Button', 'Save configuration')
    p.click_exact('Button', 'Save configuration')
    p.fill_labeled('Save configuration to', str(saved))
    p.screenshot('save-dialog')
    p.click_exact('Button', 'Save')
    p.wait('Text', 'Saved ' + str(saved))
    assert saved.is_file(), saved
    written = json.loads(saved.read_text())
    assert written['root_password'] is None, 'the configuration carries a password'
    secrets = directory / 'azfs-config.secrets.json'
    assert secrets.is_file(), 'the passwords were not written beside it'
    assert json.loads(secrets.read_text())['root_password'], secrets
    p.screenshot('configuration-saved')

    # A file with one recognisable setting proves the load replaced the state.
    other = directory / 'other.json'
    other.write_text(json.dumps({'installation_mode': 'full_disk', 'pool_name': 'fromfile'}))
    p.click_exact('Button', 'Load configuration')
    p.fill_labeled('Load configuration from', str(other))
    p.click_exact('Button', 'Save')
    p.wait('Text', 'fromfile')
    p.screenshot('configuration-loaded')


def invalid(p):
    assert not p.properties('Button', 'Install').get('accessibleEnabled', False)
    p.wait('Text', 'Complete setup before installing')
    assert p.element('Button', 'Cancel installation') is None
    p.screenshot('validation-blocks-installation')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/azfs'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sizes', nargs='+', default=SIZES)
    flows = ['system', 'users', 'desktop', 'wifi', 'inspect', 'install', 'cancel', 'shell', 'logs', 'config-file', 'invalid']
    parser.add_argument('--flows', nargs='+', choices=flows, default=flows)
    args = parser.parse_args()
    results = Results()
    for spec in args.sizes:
        size, scale = spec.split('@')
        for flow in args.flows:
            output = args.output / spec / flow
            output.mkdir(parents=True, exist_ok=True)
            scene = {'wifi': 'offline', 'install': 'review', 'cancel': 'install', 'shell': 'done', 'logs': 'done', 'config-file': 'review'}.get(flow, flow)
            preview = Preview(args.binary.resolve(), scene, size, scale, output)
            # Every captured state must also satisfy the layout invariants.
            preview.inspector = invariants.inspect
            results.run(f'{spec} {flow}', preview, globals()[flow.replace('-', '_')])
    raise SystemExit(results.finish())


if __name__ == '__main__':
    main()

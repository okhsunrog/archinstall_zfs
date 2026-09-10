#!/usr/bin/env -S uv run
"""Exercise storage decisions through the actual preview UI; never installs.

Build desktop-mock,slint/mcp with SLINT_EMIT_DEBUG_INFO=1 first.
AZFS_PREVIEW_STORAGE=empty|many|missing adds discovery edge cases (preview only).
"""
import argparse
from pathlib import Path
from review import Preview


def assign(p, label, device):
    p.click('Button', label)
    p.fill_labeled('Search storage', device)
    p.click('ListItem', device)
    p.click('Button', 'Use this device')


def storage(p):
    p.click('Button', 'Change New ZFS pool')
    p.wait('ListItem', '/dev/nvme0n1p2')
    p.screenshot('partition-picker')
    p.fill_labeled('Search storage', '/dev/sdb')
    p.wait('ListItem', '/dev/sdb1')
    assert p.properties('ListItem', '/dev/sdb1').get('accessibleEnabled', False) is False
    p.screenshot('installer-media-unavailable')
    p.fill_labeled('Search storage', '/dev/nvme0n1p1')
    p.wait('ListItem', '/dev/nvme0n1p1')
    assert p.properties('ListItem', '/dev/nvme0n1p1').get('accessibleEnabled', False) is False
    p.screenshot('efi-role-unavailable')
    p.fill_labeled('Search storage', '/dev/sda4')
    p.click('ListItem', '/dev/sda4')
    p.click('Button', 'Details')
    p.screenshot('selected-partition-details')
    p.click('Button', 'Use this device')
    p.wait('Text', '/dev/sda4')
    p.screenshot('new-pool-assigned')
    p.click('Button', 'Change EFI boot partition')
    p.fill_labeled('Search storage', '/dev/sda4')
    p.wait('ListItem', '/dev/sda4')
    assert p.properties('ListItem', '/dev/sda4').get('accessibleEnabled', False) is False
    p.key('\u001b')
    p.wait('Button', 'Change EFI boot partition')
    p.screenshot('cancel-preserves-selection')
    p.click('Button', 'ZFS')
    p.click('TextInput', 'New pool name')
    p.key('\t')
    p.key('reviewbe')
    assert p.properties('TextInput', 'Boot environment')['accessibleValue'] == 'reviewbe'
    p.wait('Text', '2 / 6')  # Tab moves through fields, not wizard steps.
    p.select('Encryption', 'Encrypt base dataset only')
    p.click('TextInput', 'Encryption passphrase')
    p.fill_labeled('Encryption passphrase', 'a preview passphrase 123 qjk')
    p.key('\t')
    p.select('Swap method', 'Swap partition (encrypted)')
    p.key('\t')
    assign(p, 'Choose Swap partition', '/dev/sda2')
    p.screenshot('zfs-encryption-swap')
    p.key('\t')
    p.click('Button', 'Additional settings')
    p.screenshot('zfs-additional-settings')
    p.click('Button', 'Review')
    p.wait('Text', 'Storage changes during installation')
    assert p.properties('Button', 'Install').get('accessibleEnabled', False) is True
    p.screenshot('new-pool-review')
    p.click('Button', 'Edit ZFS')
    p.wait('Text', '2 / 6')
    p.click('Button', 'Disk')
    p.click('RadioButton', 'Use an existing pool')
    p.wait('Button', 'Choose Existing ZFS pool')
    p.screenshot('existing-pool-empty')
    p.click('Button', 'Choose Existing ZFS pool')
    p.wait('ListItem', 'zroot')
    p.screenshot('pool-picker')
    p.click('ListItem', 'zroot')
    p.click('Button', 'Use this pool')
    assign(p, 'Choose EFI boot partition', '/dev/nvme0n1p1')
    p.click('Button', 'ZFS')
    p.click('TextInput', 'Boot environment')
    p.key('\t'); p.key('\t'); p.key('\t')
    p.select('Swap method', 'None')
    p.fill_labeled('Boot environment', 'previous')  # Existing mode has one inline name: the environment.
    p.click('Button', 'Review')
    assert p.properties('Button', 'Install').get('accessibleEnabled', False) is False
    p.screenshot('existing-environment-conflict')
    p.click('Button', 'Edit ZFS')
    p.fill_labeled('Boot environment', 'freshbe')
    p.click('Button', 'Review')
    assert p.properties('Button', 'Install').get('accessibleEnabled', False) is True
    p.screenshot('existing-pool-review')
    p.click('Button', 'Disk')
    p.click('RadioButton', 'Erase a disk')
    p.click('Button', 'Choose Disk to erase')
    p.wait('ListItem', '/dev/sdb')
    assert not p.properties('ListItem', '/dev/sdb').get('accessibleEnabled', False)
    p.screenshot('disk-picker')
    p.fill_labeled('Search storage', '/dev/sda')
    p.click('ListItem', '/dev/sda')
    p.click('Button', 'Details')
    p.screenshot('disk-details')
    p.click('Button', 'Use this device')
    p.wait('Text', '/dev/sda')
    p.screenshot('full-disk-assigned')
    p.click('Button', 'ZFS')
    p.fill_labeled('New pool name', 'newroot')
    p.click('Button', 'Review')
    assert p.properties('Button', 'Install').get('accessibleEnabled', False) is True
    p.screenshot('full-disk-review')


def keyboard(p):
    p.click('Button', 'Change EFI boot partition')
    p.fill_labeled('Search storage', '/dev/sda1')
    p.click('ListItem', '/dev/sda1')
    p.click('TextInput', 'Search storage')
    for _ in range(6):
        p.key('\t')
    p.key('no-match')
    assert 'no-match' in p.properties('TextInput', 'Search storage')['accessibleValue']
    p.wait('Text', 'No matches.')
    p.key('\u001b')
    p.key('\t')
    p.key('\n')  # Cancel returns to EFI; Tab reaches the ZFS assignment below it.
    p.wait('Text', 'Choose a ZFS partition')
    p.fill_labeled('Search storage', '/dev/sda4')
    p.key('\t'); p.key('\t')  # Focus the list, then select by arrows.
    p.key('\uf701')
    p.wait('Text', 'Selected: /dev/sda4')
    p.screenshot('keyboard-picker')
    p.key('\n')
    p.wait('Text', '/dev/sda4')
    p.screenshot('keyboard-assigned')


def edge(p, fixture):
    p.click('Button', 'Choose New ZFS pool' if fixture == 'empty' else 'Change New ZFS pool')
    if fixture == 'empty':
        p.wait('Text', 'No devices found.')
        assert p.properties('Button', 'Use this device').get('accessibleEnabled', False) is False
    elif fixture == 'many':
        p.fill_labeled('Search storage', '/dev/sda20')
        p.click('ListItem', '/dev/sda20')
        p.click('Button', 'Use this device')
        p.wait('Text', '/dev/sda20')
    else:
        p.fill_labeled('Search storage', '/dev/sda4')
        p.wait('ListItem', '/dev/sda4')
    p.screenshot('fixture-' + fixture)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--size', default='1366x768')
    parser.add_argument('--scale', default='1')
    parser.add_argument('--keyboard-only', action='store_true')
    parser.add_argument('--fixture', choices=['empty', 'many', 'missing'])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    p = Preview(Path('target/debug/azfs').resolve(), 'new-pool', args.size, args.scale, args.output)
    try:
        p.ready()
        if args.keyboard_only:
            keyboard(p)
        elif args.fixture:
            edge(p, args.fixture)
        else:
            storage(p)
        print('Storage UI flow passed', flush=True)
    except Exception:
        p.screenshot('failure')
        raise
    finally:
        p.close()


if __name__ == '__main__':
    main()

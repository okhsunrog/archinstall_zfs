#!/usr/bin/env -S uv run
"""Exercise alongside controls and review on safe preview fixtures only."""
import argparse
import os
from pathlib import Path
from review import Preview


def run(binary, output, size, scale, small):
    os.environ['AZFS_PREVIEW_ESP'] = 'small' if small else 'normal'
    p = Preview(binary, 'alongside', size, scale, output)
    prefix = f'{size}-{scale}-{ "small-esp" if small else "reuse" }'
    try:
        p.ready()
        p.reveal_by_tab('Combobox', 'Space source')
        p.screenshot(prefix + '-initial')
        p.click('Combobox', 'Space source')
        p.click('ListItem', '/dev/nvme0n1p2')
        # Shrinking proposes half of the free space (280 GiB free -> 140),
        # never everything the resizer would allow.
        p.wait_value('TextInput', 'Allocation in GiB', '140')
        p.click('TextInput', 'Allocation in GiB')
        p.fill_labeled('Allocation in GiB', '100')
        p.key('\n')
        if small:
            p.reveal_by_tab('RadioButton', 'Create a separate')
            reuse = p.properties('RadioButton', 'Reuse the selected')
            assert not reuse.get('accessibleEnabled'), reuse
            p.click('RadioButton', 'Create a separate')
        # Widgets must follow state set by Rust after the user has touched
        # them: re-selecting the source returns to the proposal, and the
        # slider tops out at the resizer's limit (450 GiB minus the 187 GiB
        # minimum), which is what a maximal drag reaches.
        p.reveal_by_tab('Combobox', 'Space source')
        p.click('Combobox', 'Space source')
        p.click('ListItem', '/dev/nvme0n1p2')
        p.wait_value('Slider', 'Space for installation', '140')
        p.wait_value('TextInput', 'Allocation in GiB', '140')
        assert abs(p.properties('Slider', 'Space for installation').get('accessibleValueMaximum', 0) - 263.0) < 0.01
        p.click('TextInput', 'Allocation in GiB')
        p.fill_labeled('Allocation in GiB', '263')
        p.key('\n')
        p.wait_value('Slider', 'Space for installation', '263')
        # A refresh keeps the current plan while the layout is unchanged.
        p.reveal_by_tab('Button', 'Refresh disks')
        p.click('Button', 'Refresh disks')
        p.wait_value('Combobox', 'Space source', '/dev/nvme0n1p2 — NTFS — 450 GiB')
        p.wait_value('TextInput', 'Allocation in GiB', '263')
        assert p.wait('Combobox', 'Space source')['accessibleValue'].startswith('/dev/nvme0n1p2'), 'refresh discarded the selected source'
        p.click('TextInput', 'Allocation in GiB')
        p.fill_labeled('Allocation in GiB', '100')
        p.key('\n')
        p.click('Button', 'Review')
        p.click('Button', 'Disk')
        p.reveal_by_tab('Combobox', 'Space source')
        p.wait('Text', '/dev/nvme0n1p2: 450 →')
        p.screenshot(prefix + '-100gib')
        p.click('Combobox', 'Space source')
        p.click('ListItem', 'Unallocated space')
        p.screenshot(prefix + '-unallocated')
        p.reveal_by_tab('Combobox', 'Swap method')
        p.click('Combobox', 'Swap method')
        p.click('ListItem', 'Swap partition')
        p.reveal_by_tab('TextInput', 'Swap size in GiB')
        p.screenshot(prefix + '-swap-controls')
        p.click('Button', 'Review')
        p.wait('Text', 'Storage changes during installation')
        p.screenshot(prefix + '-review')
        p.click('Button', 'Disk')
        p.reveal_by_tab('Combobox', 'Space source')
        p.screenshot(prefix + '-return')
        p.screenshot(prefix + '-swap-map')
        p.reveal_by_tab('RadioButton', 'Erase a disk')
        p.click('RadioButton', 'Erase a disk')
        p.reveal_by_tab('RadioButton', 'Install alongside')
        p.click('RadioButton', 'Install alongside')
        p.click('Button', 'Review')
        install = p.properties('Button', 'Install')
        assert install.get('accessibleEnabled'), install
        p.screenshot(prefix + '-mode-return')
    except Exception:
        p.screenshot(prefix + "-failure")
        raise
    finally:
        p.close()


def errors(binary, output):
    os.environ.pop('AZFS_PREVIEW_ESP', None)
    for case in ('ext4', 'missing-tools', 'no-efi', 'mbr'):
        os.environ['AZFS_PREVIEW_ALONGSIDE'] = case
        p = Preview(binary, 'alongside', '1920x1080', '1', output)
        try:
            p.ready()
            p.wait('Combobox', 'Installation disk')
            if case == 'missing-tools':
                p.click('Combobox', 'Space source')
                p.click('ListItem', '/dev/nvme0n1p2')
                p.wait('Text', 'Resizing NTFS requires')
            elif case == 'mbr':
                p.wait('Text', 'This disk does not use GPT')
            elif case == 'no-efi':
                p.wait('Text', 'No existing EFI partition')
            else:
                p.wait('Combobox', 'Space source')
            p.screenshot(case)
            if case != 'ext4':
                p.click('Button', 'Review')
                props = p.properties('Button', 'Install')
                assert not props.get('accessibleEnabled'), (case, props)
                p.screenshot(case + '-review')
        finally:
            p.close()
    os.environ.pop('AZFS_PREVIEW_ALONGSIDE', None)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/azfs'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for size, scale in [('1920x1080','1'), ('1366x768','1'), ('1920x1080','1.5'), ('1920x1080','2')]:
        for small in [False, True]:
            run(args.binary.resolve(), args.output, size, scale, small)

    errors(args.binary.resolve(), args.output)

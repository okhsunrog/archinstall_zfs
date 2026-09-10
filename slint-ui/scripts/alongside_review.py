#!/usr/bin/env -S uv run
"""Exercise alongside controls and review on safe preview fixtures only."""
import argparse
import os
from pathlib import Path
from review import Preview
from interactions import reveal_by_tab


def run(binary, output, size, scale, small):
    os.environ['AZFS_PREVIEW_ESP'] = 'small' if small else 'normal'
    p = Preview(binary, 'alongside', size, scale, output)
    prefix = f'{size}-{scale}-{ "small-esp" if small else "reuse" }'
    try:
        p.ready()
        p.wait('Combobox', 'Space source')
        p.screenshot(prefix + '-initial')
        p.click('Combobox', 'Space source')
        p.click('ListItem', '/dev/nvme0n1p2')
        reveal_by_tab(p, 'TextInput', 'Allocation in GiB')
        p.click('TextInput', 'Allocation in GiB')
        p.data('set_element_value', elementHandle=p.wait('TextInput', 'Allocation in GiB')['handle'], value='100')
        p.key('\n')
        if small:
            reveal_by_tab(p, 'Checkbox', 'Create an additional')
            p.click('Checkbox', 'Create an additional')
        p.click('Button', 'Review')
        p.click('Button', 'Disk')
        p.wait('Text', '/dev/nvme0n1p2: 450 →')
        p.screenshot(prefix + '-100gib')
        p.click('Combobox', 'Space source')
        p.click('ListItem', 'Unallocated space')
        p.screenshot(prefix + '-unallocated')
        reveal_by_tab(p, 'Combobox', 'Swap method')
        p.click('Combobox', 'Swap method')
        p.click('ListItem', 'Swap partition')
        reveal_by_tab(p, 'TextInput', 'Swap size in GiB')
        p.screenshot(prefix + '-swap-controls')
        p.click('Button', 'Review')
        p.wait('Text', 'Storage changes during installation')
        p.screenshot(prefix + '-review')
        p.click('Button', 'Disk')
        p.wait('Combobox', 'Space source')
        p.screenshot(prefix + '-return')
        p.screenshot(prefix + '-swap-map')
        p.click('RadioButton', 'Erase a disk')
        p.click('RadioButton', 'Install alongside')
        p.click('Button', 'Review')
        install = p.data('get_element_properties', elementHandle=p.wait('Button', 'Install')['handle'])
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
                props = p.data('get_element_properties', elementHandle=p.wait('Button', 'Install')['handle'])
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
    for size, scale in [('1920x1080','1'), ('1366x768','1'), ('1920x1080','1.5')]:
        for small in [False, True]:
            run(args.binary.resolve(), args.output, size, scale, small)

    errors(args.binary.resolve(), args.output)

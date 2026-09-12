#!/usr/bin/env -S uv run
"""Capture the real Slint UI on deterministic fixtures; never starts an install.

Build with SLINT_EMIT_DEBUG_INFO=1 and desktop-mock,slint/mcp first.
Run: uv run slint-ui/scripts/review.py --output /tmp/azfs-ui-review
"""
import argparse
import base64
import html
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import urllib.error
import urllib.request

SCENES = 'welcome interrupted offline no-uefi zfs-preparing zfs-failed wifi-empty wifi-unavailable wifi-no-internet wifi-verifying cancelling disk new-pool alongside existing-pool zfs system users desktop review install done failed cancelled inspect invalid'.split()

class Preview:
    def __init__(self, binary, scene, size, scale, output):
        self.output = output
        self.label = f'{scene}-{size}-{scale}x'
        self.scale = float(scale)
        self.size = tuple(map(int, size.split('x')))
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        self.log = (output / f'{self.label}.log').open('w')
        env = dict(os.environ, SLINT_BACKEND='headless', SLINT_MCP_PORT=str(self.port))
        self.process = subprocess.Popen([str(binary), '--preview', scene, '--preview-size', size, '--ui-scale', scale], env=env, stdout=self.log, stderr=subprocess.STDOUT)

    def call(self, name, **arguments):
        request = urllib.request.Request(f'http://127.0.0.1:{self.port}/mcp', data=json.dumps({'jsonrpc':'2.0', 'id':1, 'method':'tools/call', 'params': {'name':name, 'arguments':arguments}}).encode(), headers={'Content-Type':'application/json', 'Accept':'application/json, text/event-stream'})
        response = json.load(urllib.request.urlopen(request, timeout=30))
        if 'error' in response or response.get('result', {}).get('isError'):
            raise RuntimeError(response)
        return response['result']['content']

    def data(self, name, **args):
        return next(json.loads(c['text']) for c in self.call(name, **args) if c['type']=='text')

    def ready(self):
        end = time.monotonic() + 20
        while time.monotonic() < end:
            if self.process.poll() is not None:
                raise RuntimeError(f'{self.label} exited: see {self.log.name}')
            try:
                self.window = self.data('list_windows')['windowHandles'][0]
                properties = self.data('get_window_properties', windowHandle=self.window)
                self.root = properties['rootElementHandle']
                assert abs(properties['scaleFactor'] - self.scale) < .001, properties
                assert (properties['size']['width'], properties['size']['height']) == self.size, properties
                (self.output / f'{self.label}-window.json').write_text(json.dumps(properties, indent=2))
                return
            except (urllib.error.URLError, IndexError, TimeoutError):
                time.sleep(.1)
        raise TimeoutError(f'{self.label}: MCP did not start')

    def screenshot(self, label=None):
        label = label or self.label
        image = next(c for c in self.call('take_screenshot', windowHandle=self.window, imageMimeType='image/png') if c['type']=='image')
        (self.output / f'{label}.png').write_bytes(base64.b64decode(image['data']))
        tree = self.tree()
        assert not tree.get('truncated'), 'Element tree was truncated'
        (self.output / f'{label}.json').write_text(json.dumps(tree, indent=2))
        return label

    def element(self, role, label):
        tree = self.tree()
        return next((e for e in tree['elements'] if e.get('accessibleRole') == role and e.get('accessibleLabel','').startswith(label)), None)

    def tree(self):
        return self.data('get_element_tree', elementHandle=self.root, maxElements=2000)

    def fill(self, index, value):
        inputs = [e for e in self.tree()['elements'] if e.get('accessibleRole') == 'TextInput']
        self.data('set_element_value', elementHandle=inputs[index]['handle'], value=value)

    def fill_labeled(self, label, value):
        self.data('set_element_value', elementHandle=self.wait('TextInput', label)['handle'], value=value)

    def properties(self, role, label):
        return self.data('get_element_properties', elementHandle=self.wait(role, label)['handle'])

    def select(self, label, option):
        self.click('Combobox', label)
        self.click('ListItem', option)

    def key(self, text):
        self.data('dispatch_key_event', windowHandle=self.window, text=text)

    def wait(self, role, label):
        end = time.monotonic() + 15
        while time.monotonic() < end:
            element = self.element(role, label)
            if element:
                return element
            time.sleep(.1)
        raise AssertionError(f'Missing {role}: {label}')

    def click(self, role, label):
        self.data('click_element', elementHandle=self.wait(role, label)['handle'])

    def reveal_by_tab(self, role, label):
        """Navigate real focusable controls so an offscreen target scrolls into view."""
        for _ in range(50):
            e = self.element(role, label)
            if e and 100 <= e['absolutePosition']['y'] and e['absolutePosition']['y'] + e['size']['height'] < self.size[1] / self.scale - 70:
                return
            self.key('\t')
        raise AssertionError(f'Cannot reach {label} by Tab')

    def wait_value(self, role, label, value):
        """Reveal a focusable control and wait until Rust has set its value."""
        self.reveal_by_tab(role, label)
        end = time.monotonic() + 15
        while time.monotonic() < end:
            e = self.element(role, label)
            if e and e.get('accessibleValue') == value:
                return
            time.sleep(.1)
        raise AssertionError(f'{label} did not reach {value}')

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/azfs'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--scenes', nargs='+', choices=SCENES, default=SCENES)
    parser.add_argument('--sizes', nargs='+', default=['1920x1080@1', '1366x768@1', '1280x800@1', '1920x1080@1.5'])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    labels = []
    for spec in args.sizes:
        size, scale = spec.split('@')
        for scene in args.scenes:
            preview = Preview(args.binary.resolve(), scene, size, scale, args.output)
            try:
                preview.ready()
                labels.append(preview.screenshot())
                print(labels[-1], flush=True)
            finally:
                preview.close()
    cards = ''.join(f'<a href="{html.escape(label)}.png"><img src="{html.escape(label)}.png"><p>{html.escape(label)}</p></a>' for label in labels)
    (args.output / 'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Installer UI review</title><style>body{background:#181825;color:#cdd6f4;font:14px sans-serif}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(360px,1fr));gap:20px}img{width:100%}a{color:inherit;text-decoration:none}</style><h1>Installer UI review</h1><main>'+cards+'</main>')

if __name__ == '__main__':
    main()

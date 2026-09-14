#!/usr/bin/env -S uv run
"""Layout invariants every preview scene must satisfy at every display size.

Flow assertions check what someone thought to check. This walks the whole
element tree of every scene instead, so a panel that grows past the window,
a control too small to hit, an unlabelled input or two buttons drawn over
each other are caught without a flow written for that screen.

The element tree only holds what is on screen, so a control that has
scrolled away is not seen; what is checked is what the user sees.

Build desktop-mock,slint/mcp with SLINT_EMIT_DEBUG_INFO=1 first.
"""
import argparse
import json
import time
from pathlib import Path

from review import Preview, Results, SCENES, SIZES

INTERACTIVE = {'Button', 'TextInput', 'Combobox', 'Checkbox', 'ListItem', 'RadioButton', 'Slider', 'Tab', 'Switch'}
# Logical pixels. The smallest control in the design is the 28 px scroll
# button; anything under that is a layout that squeezed a control.
MIN_CONTROL = 28
# Two controls may touch; more than this much shared area in both directions
# means one is drawn over the other.
OVERLAP_SLACK = 4
EDGE_SLACK = 0.5


def names(element):
    return [entry.get('id') or entry.get('typeName', '') for entry in element.get('typeNamesAndIds', [])]


def is_popup_root(element):
    return any(name.endswith('Popup') for name in names(element))


def is_scroll_area(element):
    # An element's entries carry a type name and, for named elements, an id;
    # a ListView named `choices` has both, so both are checked.
    for entry in element.get('typeNamesAndIds', []):
        if entry.get('typeName') in ('ListView', 'Flickable', 'ScrollView'):
            return True
        if (entry.get('id') or '').endswith(('lickable', '-scroll', '-list', 'choices')):
            return True
    return False


def rect(element):
    position = element['absolutePosition']
    return position['x'], position['y'], element['size']['width'], element['size']['height']


def visible(element):
    return ('x' in element.get('absolutePosition', {}) and element.get('computedOpacity', 1) > 0
            and element['size']['width'] > 0 and element['size']['height'] > 0)


def describe(element):
    role = element.get('accessibleRole') or names(element)[0]
    label = element.get('accessibleLabel', '')
    x, y, w, h = rect(element)
    return f'{role} "{label[:40]}" at ({x:.0f},{y:.0f}) {w:.0f}x{h:.0f}'


def within(inner, outer, slack=EDGE_SLACK):
    x, y, w, h = inner
    ox, oy, ow, oh = outer
    return x >= ox - slack and y >= oy - slack and x + w <= ox + ow + slack and y + h <= oy + oh + slack


def overlap(a, b):
    ax, ay, aw, ah = a
    bx, by, bw, bh = b
    return min(ax + aw, bx + bw) - max(ax, bx), min(ay + ah, by + bh) - max(ay, by)


def check(tree, width, height):
    """Return (violations, notes) for one element tree of a width x height window."""
    violations, notes = [], []
    elements = tree['elements']
    if tree.get('truncated'):
        violations.append('element tree truncated: raise maxElements')
    window = (0, 0, width, height)

    # The tree is flat and depth-first: a popup root starts a layer and its
    # children follow it. Closed popups have no children. Only the topmost
    # open layer is interactive; the page under a popup is covered.
    layer, layers, populated = 0, [], set()
    for element in elements:
        if is_popup_root(element):
            layer += 1
        else:
            populated.add(layer)
        layers.append(layer)
    top = max(populated)
    scroll_areas = [rect(e) for e in elements if is_scroll_area(e) and visible(e)]

    # A scroll area clips its content: what falls outside it is not on
    # screen, however the element tree reports the row's own position.
    def clipped_rect(box):
        # Nested scroll areas (a popup's list over the page's scroll view)
        # each clip; intersect with every one that holds this element.
        x, y, w, h = box
        for sx, sy, sw, sh in scroll_areas:
            if sx - EDGE_SLACK <= x and x + w <= sx + sw + EDGE_SLACK:
                top_edge, bottom_edge = max(y, sy), min(y + h, sy + sh)
                y, h = top_edge, bottom_edge - top_edge
        return x, y, w, h

    def on_screen(element):
        x, y, w, h = clipped_rect(rect(element))
        return h > EDGE_SLACK and x + w > 0 and y + h > 0 and x < width and y < height

    for element in elements:
        role = element.get('accessibleRole')
        if not role or not visible(element) or not on_screen(element):
            continue
        x, y, w, h = rect(element)
        box = clipped_rect((x, y, w, h))
        if x < -EDGE_SLACK or x + w > width + EDGE_SLACK:
            violations.append(f'{describe(element)} crosses the window horizontally')
        elif not within(box, window):
            violations.append(f'{describe(element)} crosses the window vertically')
        if role in INTERACTIVE:
            if w < MIN_CONTROL or h < MIN_CONTROL:
                violations.append(f'{describe(element)} is smaller than {MIN_CONTROL} px')
            if not element.get('accessibleLabel'):
                violations.append(f'{describe(element)} has no accessible label')

    # A row half-scrolled out of its list is clipped by it, not drawn over
    # the button below, so controls are judged by their visible part.
    controls = [(clipped_rect(rect(e)), e) for i, e in enumerate(elements)
                if e.get('accessibleRole') in INTERACTIVE and visible(e) and layers[i] == top
                and on_screen(e)]
    controls = [(box, e) for box, e in controls if box[3] > OVERLAP_SLACK]
    for i, (a, ea) in enumerate(controls):
        for b, eb in controls[i + 1:]:
            if within(a, b) or within(b, a):
                continue  # a control inside a row is by design
            dx, dy = overlap(a, b)
            if dx > OVERLAP_SLACK and dy > OVERLAP_SLACK:
                violations.append(f'{describe(ea)} overlaps {describe(eb)}')

    seen = {}
    for box, element in controls:
        key = (element['accessibleRole'], element.get('accessibleLabel'))
        if key in seen:
            notes.append(f'{describe(element)} shares its label with another {key[0]}')
        seen[key] = box
    return violations, notes


def inspect(tree, preview, label):
    """Preview inspector: fail a flow's screenshot on a layout violation."""
    width, height = (side / preview.scale for side in preview.size)
    violations, _ = check(tree, width, height)
    if violations:
        raise AssertionError(f'{label}: ' + '; '.join(violations))


def scene_case(output, label):
    def case(preview):
        time.sleep(0.4)  # let entry animations settle
        tree = preview.tree()
        width, height = (side / preview.scale for side in preview.size)
        violations, notes = check(tree, width, height)
        (output / f'{label}.json').write_text(json.dumps(tree, indent=2))
        for note in notes:
            print(f'  note: {note}')
        if violations:
            preview.screenshot(label)
            raise AssertionError('; '.join(violations))
    return case


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/azfs'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--scenes', nargs='+', choices=SCENES, default=SCENES)
    parser.add_argument('--sizes', nargs='+', default=SIZES)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    results = Results()
    for spec in args.sizes:
        size, scale = spec.split('@')
        for scene in args.scenes:
            label = f'{scene}-{size}-{scale}x'
            preview = Preview(args.binary.resolve(), scene, size, scale, args.output)
            results.run(f'{spec} {scene}', preview, scene_case(args.output, label))
    raise SystemExit(results.finish())


if __name__ == '__main__':
    main()

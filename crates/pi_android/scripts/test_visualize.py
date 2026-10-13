#!/usr/bin/env python3
"""Native GPUI @Visualize checks against the isolated --ui-test APK only."""
import argparse
import json
from pathlib import Path
from test_interactions import Phone

SCROLL = 'scroll:NamedInteger("thread", 1)'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--serial', default='emulator-5554')
    parser.add_argument('--out', type=Path, default=Path('artifacts/visualize/device'))
    args = parser.parse_args()
    phone = Phone(args.serial, args.out)

    def result(value, description):
        return phone.wait(lambda s: len(s.get('visual_apps', [])) == 1 and s['visual_apps'][0]['state']['display'] == value and s['visual_apps'][0]['error'] is None, description)

    def press(action):
        name = f'visual-action-{action}'
        phone.scroll_into_view(name, SCROLL)
        phone.tap(name)

    phone.fixture('visual-calculator')
    result('0', 'complete requested bundle creates a native app')
    for action in ['7', '+', '5', '=']:
        press(action)
    result('12', 'native touches dispatch JavaScript arithmetic and update GPUI')
    phone.capture('01-native-calculator-moonstone')
    phone.scroll_into_view('visual-source', SCROLL, 0)
    phone.tap('visual-source', 0)
    phone.wait(lambda s: s['sheet'] == 'SelectText', 'source is selectable in a native sheet')
    result('12', 'opening source retains calculator state')
    phone.key('KEYCODE_BACK')
    phone.wait(lambda s: s['sheet'] is None, 'back closes source')
    result('12', 'closing source retains calculator state')
    phone.scroll_into_view('visual-reset', SCROLL, 0)
    phone.tap('visual-reset', 0)
    result('0', 'explicit reset restores initial app state')

    for fixture in ['visual-calculator-partial', 'visual-calculator-invalid', 'visual-calculator-unrequested']:
        phone.fixture(fixture)
        phone.wait(lambda s: not s.get('visual_apps'), f'{fixture} stays inert code')
        assert not any(key.startswith('visual-action-') for key in phone.state()['bounds'])

    phone.fixture('visual-calculator-evening')
    for action in ['1', '2', '8', '*', '3', '=']:
        press(action)
    result('384', 'the same native app works in Evening')
    phone.capture('02-native-calculator-evening')
    phone.output.joinpath('report.json').write_text(json.dumps({'renderer':'native GPUI; no WebView app surface', 'checks':phone.results}, indent=2) + '\n')


if __name__ == '__main__':
    main()

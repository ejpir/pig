#!/usr/bin/env python3
"""Native canvas/slider/motion smoke checks, isolated fixture APK only."""
import argparse
import json
import time
from pathlib import Path
from test_interactions import Phone

SCROLL = 'scroll:NamedInteger("thread", 1)'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--serial', default='emulator-5554')
    parser.add_argument('--out', type=Path, default=Path('artifacts/visualize-v2/device'))
    args = parser.parse_args()
    phone = Phone(args.serial, args.out)

    def app(state=None):
        state = state or phone.state()
        apps = state.get('visual_apps', [])
        assert len(apps) == 1 and apps[0]['error'] is None and not apps[0].get('drawing_failed', False), apps
        return apps[0]['state']

    def reveal(name, index=None):
        # Slider/canvas gestures own their contacts. Scroll in the conversation
        # margin instead of starting a pan on a direct-manipulation control.
        for _ in range(10):
            state = phone.settled_state()
            (_, y, _, h), _ = phone.bounds(name, index, state)
            (sx, sy, _, sh), _ = phone.bounds(SCROLL, state=state)
            top, bottom = sy + 8, sy + sh - 8
            if top <= y and y + h <= bottom:
                return
            distance = min(max(top-y if y < top else y+h-bottom, 48), (bottom-top)*.6)
            if y < top:
                phone.swipe((sx+4, top), (sx+4, top+distance))
            else:
                phone.swipe((sx+4, bottom), (sx+4, bottom-distance))
        raise AssertionError(f"Could not reveal {name}")

    def press(action):
        name = f'visual-action-{action}'
        reveal(name)
        phone.tap(name)

    def canvas_visible():
        names = [n for n in phone.state()['bounds'] if n.startswith('visual-canvas-')]
        assert len(names) == 1, names
        reveal(names[0])

    original_scale = phone.run('shell', 'settings', 'get', 'global', 'animator_duration_scale').decode().strip()
    phone.run('shell', 'settings', 'put', 'global', 'animator_duration_scale', '1')
    try:
        phone.fixture('visual-spirograph')
        phone.wait(lambda s: len(s.get('visual_apps', [])) == 1, 'v2 native spirograph loads')
        assert not app()['playing'] and app()['angle'] == 0.0
        canvas_visible()
        phone.capture('01-native-spirograph-paused')
        press('play')
        canvas_visible()
        phone.wait(lambda s: app(s)['playing'] and app(s)['angle'] > 0.15, 'native ticks move the rolling circle and pen')
        phone.capture('02-native-spirograph-playing')
        press('play')
        phone.wait(lambda s: not app(s)['playing'], 'Play / Pause stops animation')
        paused = app()['angle']
        time.sleep(0.6)
        assert app()['angle'] == paused, 'paused state must not advance'
        phone.results.append({'check': 'paused angle remains stable', 'passed': True})
        press('restart')
        phone.wait(lambda s: app(s)['angle'] == 0.0, 'Restart rewinds the pen')

        reveal('visual-slider-ring')
        (x, y, width, height), _ = phone.bounds('visual-slider-ring')
        phone.swipe((x + 12 + (width - 24) * .3, y + height / 2),
                    (x + 12 + (width - 24) * .75, y + height / 2), 500)
        phone.wait(lambda s: 115 <= app(s)['ring'] <= 125 and app(s)['ring_label'] == f"Fixed ring · {int(app(s)['ring'])}",
                   'native slider updates numeric state, readout and curve')
        assert 115 <= app()['ring'] <= 125, app()

        press('play')
        canvas_visible()
        start = app()['angle']
        phone.wait(lambda s: app(s)['angle'] > start + .1, 'animation resumes after slider changes')
        reveal('visual-source', 0)
        phone.tap('visual-source', 0)
        phone.wait(lambda s: s['sheet'] == 'SelectText', 'Source opens without replacing the runtime')
        time.sleep(.3)
        held = app()['angle']
        time.sleep(.6)
        assert app()['angle'] == held, 'overlay must suspend animation'
        phone.results.append({'check': 'source overlay suspends ticks', 'passed': True})
        phone.key('KEYCODE_BACK')
        phone.wait(lambda s: s['sheet'] is None, 'Source closes')
        canvas_visible()
        phone.wait(lambda s: app(s)['angle'] != held, 'visible canvas resumes after closing Source')

        phone.fixture('visual-spirograph-evening')
        canvas_visible()
        phone.capture('03-native-spirograph-evening')
        press('play')
        canvas_visible()
        phone.wait(lambda s: app(s)['angle'] > .15, 'canvas animates in Evening')
        # Android Remove animations must feed GPUI's policy, not just test flags.
        phone.run('shell', 'settings', 'put', 'global', 'animator_duration_scale', '0')
        time.sleep(.8)
        held = app()['angle']
        time.sleep(.6)
        assert app()['angle'] == held, 'system reduced motion must suspend ticks'
        phone.results.append({'check': 'Android Remove animations suspends native motion', 'passed': True})
        phone.run('shell', 'settings', 'put', 'global', 'animator_duration_scale', '1')
        phone.wait(lambda s: app(s)['angle'] != held, 'restoring system motion resumes visible animation')
        press('reset')
        phone.wait(lambda s: not app(s)['playing'] and app(s)['ring'] == 96.0 and app(s)['angle'] == 0.0,
                   'Reset restores controls and stops motion')
        phone.fixture('visual-spirograph-trace')
        canvas_visible()
        press('play')
        canvas_visible()
        phone.wait(lambda s: app(s)['progress'] == 1.0 and not app(s)['playing'],
                   'growing trace renders through 100% at display-driven rate', timeout=20)
        phone.capture('04-native-complete-trace')
        # Repeated rapid reversals: endpoint delivery, quantization, and recovery.
        reveal('visual-slider-ring')
        (x, y, width, height), _ = phone.bounds('visual-slider-ring')
        for start, end in [(.2,.8),(.8,.2),(.2,.8),(.8,.2)]:
            phone.swipe((x+12+(width-24)*start,y+height/2),
                        (x+12+(width-24)*end,y+height/2),180)
        phone.wait(lambda s: 72 <= app(s)['ring'] <= 80,
                   'rapid slider reversals deliver the final position without drawing faults')
        phone.output.joinpath('report.json').write_text(json.dumps({'renderer': 'native GPUI paths; no WebView', 'checks': phone.results}, indent=2) + '\n')
    finally:
        if original_scale == 'null':
            phone.run('shell', 'settings', 'delete', 'global', 'animator_duration_scale')
        else:
            phone.run('shell', 'settings', 'put', 'global', 'animator_duration_scale', original_scale)


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Aggregate timing checks in isolated sample sessions only, not displayed FPS."""
import argparse
import json
import time
from pathlib import Path
from test_interactions import Phone

SCROLL = 'scroll:NamedInteger("thread", 1)'


def reveal(phone, name):
    # Scroll in the thread margin, not on a gesture-owning slider/canvas.
    for _ in range(12):
        state = phone.settled_state()
        (_, y, _, height), _ = phone.bounds(name, state=state)
        (sx, sy, _, sh), _ = phone.bounds(SCROLL, state=state)
        top, bottom = sy + 8, sy + sh - 8
        if top <= y and y + height <= bottom:
            return
        distance = min(max(top - y if y < top else y + height - bottom, 48), (bottom - top) * .6)
        phone.swipe((sx + 4, top if y < top else bottom),
                    (sx + 4, top + distance if y < top else bottom - distance))
    raise AssertionError(f"Could not reveal {name}")


def toggle_stats(phone):
    phone.tap('Name("more")')
    phone.wait(lambda s: s['sheet'] == 'More(SessionId(1))', 'isolated sample menu opens')
    state = phone.settled_state()
    # Known previous v3 APK has this exact sample menu ordering; new APK has a
    # named probe. No production sessions or arbitrary menu ordinals are used.
    name = 'rendering-stats-toggle' if 'rendering-stats-toggle' in state['bounds'] else 'NamedInteger("action", 3)'
    phone.tap(name)
    phone.wait(lambda s: s['sheet'] is None, 'isolated sample menu closes')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--serial', default='emulator-5554')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline', action='store_true')
    args = parser.parse_args()
    phone = Phone(args.serial, args.out)
    phone.fixture('visual-spirograph')
    if phone.state().get('visual_profile', {}).get('enabled', False):
        toggle_stats(phone)
    reveal(phone, 'visual-action-play')
    phone.tap('visual-action-play')
    phone.wait(lambda s: s['visual_apps'][0]['state']['playing'], 'sample animation starts')
    toggle_stats(phone)
    phone.wait(lambda s: s['visual_profile']['enabled'], 'aggregate profiling enabled')
    canvas = next(n for n in phone.state()['bounds'] if n.startswith('visual-canvas-'))
    reveal(phone, canvas)
    time.sleep(4)
    state = phone.state()
    assert state['visual_apps'][0]['error'] is None and not state['visual_apps'][0]['drawing_failed']
    profile = state['visual_profile']
    assert profile['scene_misses'] >= 30, profile
    if not args.baseline:
        assert profile['text_vm_p95_ms'] > 0 and profile['native']['draws'] >= 30, profile
        # A bounded naming check prevents mistaking service/tick rates for
        # presentation acknowledgements or a physical display FPS measurement.
        assert profile['native']['present_requests'] <= profile['native']['draws']
    phone.output.joinpath('animation-profile.json').write_text(json.dumps(profile, indent=2) + '\n')
    phone.capture('01-animation-profile')
    reveal(phone, 'visual-action-play')
    phone.tap('visual-action-play')
    phone.wait(lambda s: not s['visual_apps'][0]['state']['playing'], 'sample animation pauses')
    toggle_stats(phone)
    reveal(phone, 'visual-slider-ring')
    toggle_stats(phone)
    (x, y, width, height), _ = phone.bounds('visual-slider-ring')
    for start, stop in [(0.2, .8), (.8, .2), (.2, .8), (.8, .6)]:
        phone.swipe((x + 12 + (width - 24) * start, y + height / 2),
                    (x + 12 + (width - 24) * stop, y + height / 2), 600)
    phone.wait(lambda s: s['visual_profile']['delivered'] > 10, 'coalesced sample moves are profiled')
    state = phone.state()
    assert state['visual_apps'][0]['error'] is None and not state['visual_apps'][0]['drawing_failed']
    phone.output.joinpath('drag-profile.json').write_text(json.dumps(state['visual_profile'], indent=2) + '\n')
    phone.capture('02-drag-profile')
    toggle_stats(phone)
    phone.output.joinpath('report.json').write_text(json.dumps({'renderer': 'native GPUI',
        'measurement': 'bounded invocation/root/backend wall times, not presentation FPS',
        'checks': phone.results}, indent=2) + '\n')


if __name__ == '__main__':
    main()

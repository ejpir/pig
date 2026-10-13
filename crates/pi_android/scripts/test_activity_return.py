#!/usr/bin/env python3
"""Isolated fixture activity recreation; never installs or touches main app data."""
import argparse
import json
import re
import time
from pathlib import Path
from test_interactions import Phone, PACKAGE, ACTIVITY


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--serial', required=True)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--cycles', type=int, default=5)
    args = parser.parse_args()
    if not 1 <= args.cycles <= 12:
        parser.error('cycles must be 1–12')
    phone = Phone(args.serial, args.out)
    phone.fixture('visual-spirograph')

    def pid():
        value = phone.run('shell', 'pidof', PACKAGE).decode().strip()
        assert value.isdecimal(), 'fixture must have exactly one process'
        return value

    def resources(process):
        # Fixture UID only, OS thread/descriptor metadata, no process memory.
        command = f'for f in /proc/{process}/task/*/comm; do cat "$f"; done'
        names = phone.run('exec-out', 'run-as', PACKAGE, 'sh', '-c', command).decode().splitlines()
        return {'vsync': names.count('Vsync'), 'timers': names.count('Timer'),
                'workers': sum(n.startswith('Worker-') for n in names), 'threads': len(names)}

    def activity():
        text = phone.run('shell', 'dumpsys', 'activity', 'activities').decode()
        match = re.search(r'topResumedActivity=ActivityRecord\{([^ ]+) [^\n]*' + re.escape(PACKAGE) + r'/', text)
        assert match, 'isolated fixture must be resumed'
        return match.group(1)

    def returns(process):
        text = phone.run('logcat', '-d', '--pid=' + process).decode()
        return text.count('Native activity loop returned; returning to Android glue')

    process = pid()
    identity = activity()
    returned = returns(process)
    time.sleep(1)
    baseline = resources(process)
    assert baseline['vsync'] == baseline['timers'] == 1, baseline
    records = [{'cycle': 'initial', 'pid': process, 'resources': baseline}]
    for cycle in range(args.cycles):
        previous = phone.state()['time']
        # NEW_TASK|CLEAR_TASK affects only the explicitly named isolated package.
        result = phone.run('shell', 'am', 'start', '-f', '0x10008000', '-n', ACTIVITY,
                           '-a', 'android.intent.action.VIEW', '-d', 'pi://preview/visual-spirograph')
        args.out.joinpath(f'recreate-{cycle}.txt').write_bytes(result)
        phone.wait(lambda s: s['fixture'] == 'visual-spirograph' and s['time'] > previous
                   and activity() != identity and returns(process) > returned,
                   'new Activity receives startup intent after old native return')
        identity = activity()
        returned = returns(process)
        assert pid() == process, 'normal activity return should preserve this process'
        state = phone.settled_state()
        assert state['visual_apps'][0]['error'] is None and not state['visual_apps'][0]['drawing_failed']
        time.sleep(1)
        current = resources(process)
        assert all(current[k] == baseline[k] for k in ('vsync', 'timers', 'workers')), (baseline, current)
        records.append({'cycle': cycle, 'pid': process, 'activity': identity,
                        'native_returns': returned, 'resources': current})
        args.out.joinpath('relaunch.json').write_text(json.dumps(records, indent=2) + '\n')
    # Bound telemetry belongs to this fixture; Phone checks foreground before
    # every injection, so these coordinates can never target main/another app.
    phone.scroll_into_view('visual-slider-ring', 'scroll:NamedInteger("thread", 1)')
    x, y, width, height = phone.bounds('visual-slider-ring')[0]
    previous_ring = phone.state()['visual_apps'][0]['state']['ring']
    phone.swipe((x + width * .25, y + height / 2), (x + width * .65, y + height / 2), 400)
    state = phone.wait(lambda s: s['visual_apps'][0]['state']['ring'] != previous_ring,
                       'native slider actually changes state after recreation')
    assert state['visual_apps'][0]['error'] is None and not state['visual_apps'][0]['drawing_failed']
    args.out.joinpath('report.json').write_text(json.dumps(phone.results, indent=2) + '\n')
    print(f'PASS {args.cycles} same-process recreations; stable owned worker/timer/vsync counts; native input remains valid')


if __name__ == '__main__':
    main()

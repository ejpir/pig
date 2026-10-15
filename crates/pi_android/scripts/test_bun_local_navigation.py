#!/usr/bin/env python3
"""Emulator-only: stock Local tool survives parking behind another backend; Stop still works.

Uses the sample backend to exercise the same replace_store/parked Local path as
SSH without opening a real SSH connection or using any real provider account.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

PACKAGE = 'dev.pi.android.quickjstest'
ACTIVITY = PACKAGE + '/dev.pi.gpui.GpuiActivity'
WORKSPACE = 'no_backup/local-agent/synthetic/workspace/'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adb', default='adb')
    parser.add_argument('--serial', default='emulator-5554')
    parser.add_argument('--output', default='artifacts/bun-local-navigation-emulator')
    args = parser.parse_args()
    if not args.serial.startswith('emulator-'):
        raise SystemExit('This regression operates only on an emulator, never the user phone')
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=True)

    def run(*parts, check=True):
        return subprocess.run([args.adb, '-s', args.serial, *parts], check=check,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)

    def state():
        try:
            data = json.loads(run('shell', 'run-as', PACKAGE, 'cat',
                                  'files/quickjs-chat-test/ui.json').stdout)
            assert data['synthetic'] is True
            return data
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            return {}

    def wait(predicate, seconds=30):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            value = state()
            if value and predicate(value):
                return value
            time.sleep(.08)
        raise AssertionError('Synthetic UI timeout: ' + json.dumps(state()))

    def link(url):
        run('shell', 'am', 'start', '-W', '-a', 'android.intent.action.VIEW',
            '-d', url, '-n', ACTIVITY)
        time.sleep(.5)

    def tap(name):
        time.sleep(.6)
        value = wait(lambda s: s['visible'] and name in s.get('bounds', {}))
        x, y, w, h = value['bounds'][name]
        scale = value['scale']
        run('shell', 'input', 'tap', str(round((x + w / 2) * scale)), str(round((y + h / 2) * scale)))

    def file_exists(name):
        return run('shell', 'run-as', PACKAGE, 'test', '-f', WORKSPACE + name, check=False).returncode == 0

    def start_tool():
        for name in ['navigation-tool-started', 'navigation-tool-finished']:
            run('shell', 'run-as', PACKAGE, 'rm', '-f', WORKSPACE + name)
        tap('draft')
        time.sleep(.7)
        run('shell', 'input', 'text', 'offline%stools%sslow')
        tap('send')
        end = time.monotonic() + 20
        while not file_exists('navigation-tool-started'):
            if time.monotonic() > end:
                raise AssertionError('Stock SDK bash tool did not start: ' + json.dumps(state()))
            time.sleep(.1)
        wait(lambda s: s['running'])

    link('pi://test/local-providers/synthetic')
    run('shell', 'input', 'keyevent', 'KEYCODE_BACK')
    link('pi://test/local-backend/synthetic')
    wait(lambda s: s['visible'] and s['route'] == 'Sessions')
    tap('new-session')
    wait(lambda s: s['route'] == 'Start')
    start_tool()
    pid = state()['pid']
    # Preview Sessions installs the sample Store through the normal backend seam.
    link('pi://preview/sessions')
    assert int(run('shell', 'pidof', PACKAGE).stdout.split()[0]) == pid
    time.sleep(10)
    assert file_exists('navigation-tool-finished'), 'Navigation aborted the running stock tool'
    link('pi://test/local-backend/synthetic')
    value = wait(lambda s: s['visible'] and s['route'] == 'Sessions')
    rows = [name for name in value['bounds'] if name.startswith('session-row-')]
    assert rows, value
    latest = max(rows, key=lambda name: int(name.removeprefix('session-row-')))
    tap(latest)
    completed = wait(lambda s: not s['running'] and s.get('tool_verified', False))
    (out / 'completed.json').write_text(json.dumps(completed, indent=2))
    (out / 'completed.png').write_bytes(run('exec-out', 'screencap', '-p').stdout)
    # A second real tool proves explicit Stop remains effective after returning.
    start_tool()
    tap('stop')
    stopped = wait(lambda s: not s['running'])
    time.sleep(9)
    assert not file_exists('navigation-tool-finished'), 'Explicit Stop failed to cancel the tool'
    (out / 'stopped.json').write_text(json.dumps(stopped, indent=2))
    result = {'synthetic': True, 'pid': pid, 'toolCompletedWhileParked': True,
              'returnedToSameLocalSession': True, 'explicitStopCancelledTool': True,
              'otherBackend': 'sample (same Store parking path as SSH)'}
    (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Passive, opt-in timing capture. Never launches apps, injects input or installs.

No run-as reads, source/state/coordinates or task source locations are recorded.
The system trace does contain process/thread names and graphics-layer metadata;
keep captures local. Each capture uses a unique remote filename, including when
an earlier capture was interrupted. Config is sent on stdin to avoid SELinux.
"""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import shutil
from string import Template
import subprocess
import time
import uuid

PACKAGES = ('dev.pi.android', 'dev.pi.android.uitest')


def config(package, seconds):
    if package not in PACKAGES or not 3 <= seconds <= 30:
        raise ValueError('Use the main/fixture package and a 3..30 second interval')
    text = Path(__file__).with_name('visual_trace.pbtxt').read_text()
    return Template(text).substitute(package=package, duration_ms=seconds * 1000)


def remote_path():
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    return f'/data/misc/perfetto-traces/pi-visual-{stamp}-{uuid.uuid4().hex[:8]}.perfetto-trace'


def resumed_package(text, package):
    return any(('topResumedActivity' in line or 'mResumedActivity' in line)
               and re.search(r'\b' + re.escape(package) + r'/', line) for line in text.splitlines())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--package', choices=PACKAGES, default=PACKAGES[0])
    parser.add_argument('--seconds', type=int, default=15)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--collect', help='Collect a previous interrupted capture using its remote path; do not start another')
    args = parser.parse_args()
    adb = shutil.which('adb')
    if not adb:
        raise RuntimeError('Put Android SDK platform-tools on PATH')
    command = [adb, '-s', args.serial]
    args.out.mkdir(parents=True, exist_ok=True)
    if (args.out / 'trace.perfetto-trace').exists():
        raise RuntimeError('Use a fresh output directory; do not overwrite an existing capture')

    def run(*argv, **kwargs):
        return subprocess.check_output(command + list(argv), stderr=subprocess.STDOUT,
                                       timeout=40, **kwargs)

    if args.collect:
        remote = args.collect
        if not re.fullmatch(r'/data/misc/perfetto-traces/pi-visual-[A-Za-z0-9-]+\.perfetto-trace', remote):
            raise ValueError('Collect only a capture path produced by this script')
    else:
        trace_config = config(args.package, args.seconds)
        activity = run('shell', 'dumpsys', 'activity', 'activities').decode()
        if not resumed_package(activity, args.package):
            raise RuntimeError('Selected app must already be foreground. No app was launched or touched.')
        try:
            pid = run('shell', 'pidof', args.package).decode().strip()
        except subprocess.CalledProcessError as error:
            raise RuntimeError('Selected process is not running; no trace was started.') from error
        remote = remote_path()
        (args.out / 'trace-config.pbtxt').write_text(trace_config)
        (args.out / 'capture.json').write_text(json.dumps({'package': args.package,
            'serial': args.serial, 'pid': pid, 'seconds': args.seconds, 'remote': remote,
            'host_start_utc': datetime.now(timezone.utc).isoformat()}, indent=2) + '\n')
        output = run('shell', 'perfetto', '--background-wait', '--txt', '-c', '-', '-o', remote,
                     input=trace_config.encode())
        (args.out / 'capture-start.txt').write_bytes(output)
        print(f'CAPTURING NOW: {args.seconds}s; keep dragging if testing input.', flush=True)
        print(f'Remote file (safe to collect after interruption): {remote}', flush=True)
        time.sleep(args.seconds + 2)

    size = int(run('shell', 'stat', '-c', '%s', remote).decode().strip())
    if size <= 0:
        raise RuntimeError('Trace is empty or not finished; do not automatically repeat the interaction.')
    trace = args.out / 'trace.perfetto-trace'
    partial = args.out / 'trace.perfetto-trace.partial'
    output = run('pull', remote, str(partial))
    (args.out / 'capture-pull.txt').write_bytes(output)
    if partial.stat().st_size != size:
        raise RuntimeError('Pulled trace size differs from device file; partial file retained')
    # A streaming ADB pull creates its destination before it has finished.
    # Publish the final pathname only after completion and size verification.
    partial.replace(trace)
    print(f'SAVED {size} bytes to {trace}. Analyze retention/markers before drawing conclusions.', flush=True)


if __name__ == '__main__':
    main()

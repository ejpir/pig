#!/usr/bin/env python3
"""Pure configuration/selection checks: no device or trace is started."""
import unittest
from pathlib import Path
import tempfile
from unittest.mock import patch
import capture_visual_trace as capture
from test_interactions import Phone

class TraceCaptureTests(unittest.TestCase):
    def test_buffers_streaming_metadata_and_correct_selected_package(self):
        text = capture.config('dev.pi.android', 15)
        self.assertIn('size_kb: 65536', text)
        self.assertIn('target_buffer: 1', text)
        self.assertIn('record_thread_names: true', text)
        self.assertIn('write_into_file: true', text)
        self.assertIn('duration_ms: 15000', text)
        self.assertIn('atrace_apps: "dev.pi.android"', text)
        self.assertNotIn('$', text)

    def test_scope_and_duration_are_bounded(self):
        for package, seconds in [('other.app', 15), ('dev.pi.android', 0), ('dev.pi.android', 31)]:
            with self.assertRaises(ValueError):
                capture.config(package, seconds)

    def test_remote_names_never_overwrite_an_interrupted_capture(self):
        self.assertNotEqual(capture.remote_path(), capture.remote_path())
        self.assertTrue(capture.remote_path().startswith('/data/misc/perfetto-traces/pi-visual-'))

    def test_stale_fixture_coordinates_cannot_inject_input_into_main_app(self):
        with tempfile.TemporaryDirectory() as directory:
            phone = Phone('fake', Path(directory))
            with patch('test_interactions.subprocess.check_output', return_value=
                       b'topResumedActivity=ActivityRecord{dev.pi.android/dev.pi.gpui.GpuiActivity}') as call:
                with self.assertRaises(AssertionError):
                    phone.run('shell', 'input', 'tap', 10, 10)
                self.assertEqual(call.call_count, 1)  # only foreground query
            with patch('test_interactions.subprocess.check_output', side_effect=[
                       b'topResumedActivity=ActivityRecord{dev.pi.android.uitest/dev.pi.gpui.GpuiActivity}',
                       b'mCurrentFocus=Window{fixture dev.pi.android.uitest/dev.pi.gpui.GpuiActivity}',
                       b'']) as call:
                phone.run('shell', 'input', 'tap', 10, 10)
                self.assertEqual(call.call_count, 3)  # resumed Activity, focused window, then input

    def test_collection_publishes_only_a_completed_verified_pull(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            remote = '/data/misc/perfetto-traces/pi-visual-20261013T190322Z-32a99300.perfetto-trace'

            def adb_command(argv, **_):
                if 'stat' in argv:
                    return b'4\n'
                self.assertIn('pull', argv)
                self.assertFalse((out / 'trace.perfetto-trace').exists())
                partial = Path(argv[-1])
                self.assertEqual(partial.name, 'trace.perfetto-trace.partial')
                partial.write_bytes(b'test')
                return b'pulled'

            with patch('sys.argv', ['capture', '--serial', 'fake', '--out', str(out), '--collect', remote]), \
                 patch('capture_visual_trace.shutil.which', return_value='/fake/adb'), \
                 patch('capture_visual_trace.subprocess.check_output', side_effect=adb_command):
                capture.main()
            self.assertEqual((out / 'trace.perfetto-trace').read_bytes(), b'test')
            self.assertFalse((out / 'trace.perfetto-trace.partial').exists())

    def test_main_and_fixture_foreground_checks_are_disjoint(self):
        state = 'topResumedActivity=ActivityRecord{abc u0 dev.pi.android.uitest/dev.pi.gpui.GpuiActivity}'
        self.assertTrue(capture.resumed_package(state, 'dev.pi.android.uitest'))
        self.assertFalse(capture.resumed_package(state, 'dev.pi.android'))
        self.assertFalse(capture.resumed_package('old dev.pi.android/dev.pi.gpui.GpuiActivity', 'dev.pi.android'))

if __name__ == '__main__':
    unittest.main()

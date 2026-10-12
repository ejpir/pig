#!/usr/bin/env python3
"""Tooling-only safety tests; fake ADB, no emulator, provider or Rust builds."""
import json
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

import test_schedules as schedules


def sample(**changes):
    state = {
        "fixture": "schedules", "sample": True, "time": time.time(),
        "bounds": {"schedule-new": [10, 10, 100, 48]}, "viewport": [360, 640], "scale": 2,
        "sessions": [1, 2], "scheduler": {"mode": "sample", "definitions": [
            {"id": "review", "enabled": True, "revision": 1}], "occurrences": []},
    }
    state.update(changes)
    return state


class SafetyTests(unittest.TestCase):
    def test_sample_boundary_and_freshness(self):
        state = sample()
        self.assertIs(schedules.require_sample(state, "schedules"), state)
        bad_states = [sample(sample=False), sample(sample=1), sample(fixture="sessions"),
                      sample(time=time.time() - 10), sample(time=time.time() + 10),
                      sample(time=float("nan")), sample(scheduler={"mode": "live"})]
        for state in bad_states:
            with self.subTest(state=state), self.assertRaises(AssertionError):
                schedules.require_sample(state, "schedules")
        with self.assertRaises(AssertionError):
            schedules.require_sample(sample(), "not-approved")

    def test_device_clock_skew_is_calibrated_without_changing_device_time(self):
        now = time.time()
        state = sample(time=now - 9)
        self.assertIs(schedules.require_sample(state, "schedules", now=now - 9), state)
        with self.assertRaises(AssertionError):
            schedules.require_sample(state, "schedules", now=now)

    def test_scroll_clipping_excludes_controls_behind_appbar_or_dock(self):
        state = sample(bounds={"schedule-consent": [20, 550, 300, 64],
                               'scroll:Name("schedule-confirm-scroll")': [0, 90, 360, 480]})
        self.assertFalse(schedules.visible(state, "schedule-consent"))
        state["bounds"]["schedule-consent"] = [20, 400, 300, 64]
        self.assertTrue(schedules.visible(state, "schedule-consent"))

    def test_consent_checks_the_wrapped_label_not_only_the_row(self):
        state = sample(bounds={"schedule-consent": [20, 400, 320, 98],
                               "schedule-consent-label": [72, 416, 252, 66],
                               'scroll:Name("schedule-confirm-scroll")': [0, 90, 360, 480]})
        self.assertTrue(schedules.consent_fits(state))
        for label in ([72, 416, 270, 66], [72, 416, 252, 22], [72, 416, 252, 90],
                      [72, 404, 252, 66], [50, 416, 252, 66]):
            with self.subTest(label=label):
                state["bounds"]["schedule-consent-label"] = label
                self.assertTrue(schedules.visible(state, "schedule-consent"))
                self.assertFalse(schedules.consent_fits(state))

    def test_delivered_index_selectors_are_mapped_from_stable_ids(self):
        state = sample()
        state["scheduler"]["selected_id"] = "review"
        state["scheduler"]["occurrences"] = [
            {"id": "other-job", "job_id": "other"},
            {"id": "run-0", "job_id": "review"}, {"id": "run-1", "job_id": "review"}]
        self.assertEqual(schedules.schedule_row(state, "review"), "schedule-row-0")
        self.assertEqual(schedules.history_link(state, "run-1"), "schedule-session-1")

    def test_bounds_are_unique_finite_and_fully_visible(self):
        state = sample()
        self.assertTrue(schedules.visible(state, "schedule-new"))
        for bounds in ([10, 620, 100, 48], [-5, 10, 100, 48], [10, 10, 0, 48],
                       [10, 10, float("nan"), 48], [300, 10, 100, 48]):
            state["bounds"]["schedule-new"] = bounds
            self.assertFalse(schedules.visible(state, "schedule-new"))
        self.assertFalse(schedules.visible(state, "missing"))
        state["bounds"] = {'Name("schedule-new")': [10, 10, 100, 48]}
        self.assertTrue(schedules.visible(state, "schedule-new"))
        state["bounds"]["schedule-new"] = [10, 10, 100, 48]
        with self.assertRaises(AssertionError):
            schedules.control_rect(state, "schedule-new")

    def test_every_inherited_input_rechecks_sample(self):
        with tempfile.TemporaryDirectory() as directory:
            phone = schedules.SchedulerPhone("emulator-5554", Path(directory), "/fake/adb")
            phone.fixture_name = "schedules"
            with patch.object(phone, "raw_state", return_value=sample(sample=False)), \
                    patch.object(schedules.subprocess, "run") as run:
                for arguments in (("shell", "input", "tap", 1, 2),
                                  ("shell", "input", "keyevent", "KEYCODE_BACK"),
                                  ("shell", "input", "swipe", 1, 2, 3, 4),
                                  ("shell", "input", "text", "Hello")):
                    with self.assertRaises(AssertionError):
                        phone.run(*arguments)
                run.assert_not_called()

    def test_unapproved_fixture_cannot_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            phone = schedules.SchedulerPhone("emulator-5554", Path(directory), "/fake/adb")
            with patch.object(phone, "run") as run:
                with self.assertRaises(ValueError):
                    phone.fixture("working")
                run.assert_not_called()

    def test_real_preferences_are_refused_without_clear_or_force_stop(self):
        with tempfile.TemporaryDirectory() as directory:
            phone = schedules.SchedulerPhone("emulator-5554", Path(directory), "/fake/adb")
            with patch.object(phone, "read_settings", return_value=b'{"computer":"real-host","sample":false}'), \
                    patch.object(phone, "run", return_value=b"") as run:
                with self.assertRaises(RuntimeError):
                    phone.preflight()
                self.assertEqual(run.call_args_list[0].args, ("shell", "pm", "path", schedules.PACKAGE))
                self.assertEqual(run.call_count, 1)

    def test_exact_command_and_failure_logged(self):
        with tempfile.TemporaryDirectory() as directory:
            phone = schedules.SchedulerPhone("emulator-5554", Path(directory), "/fake/adb")
            failure = subprocess.CompletedProcess([], 1, b"not installed")
            with patch.object(schedules.subprocess, "run", return_value=failure):
                with self.assertRaises(subprocess.CalledProcessError):
                    phone.run("shell", "pm", "path", schedules.PACKAGE)
            entry = json.loads(phone.commands.read_text())
            self.assertEqual(entry["argv"], ["/fake/adb", "-s", "emulator-5554", "shell", "pm", "path", schedules.PACKAGE])
            self.assertEqual(entry["returncode"], 1)
            self.assertIn("not installed", entry["error"])

    def test_snapshots_include_definitions_occurrences_and_sessions(self):
        state = sample()
        snapshot = schedules.mutation_snapshot(state)
        self.assertEqual(set(snapshot), {"definitions", "occurrences", "sessions"})
        self.assertEqual(schedules.definitions(state)["review"]["revision"], 1)
        self.assertEqual(schedules.occurrences(state), [])

    def test_native_geometry_performs_no_adb_commands(self):
        with schedules.geometry(None, "native"):
            pass

    def test_geometry_refuses_physical_devices(self):
        class Device:
            serial = "real-phone"
        with self.assertRaises(RuntimeError):
            with schedules.geometry(Device(), "small"):
                self.fail("physical device geometry was changed")

    def test_geometry_restores_prior_overrides_on_interruption(self):
        with tempfile.TemporaryDirectory() as directory:
            class Device:
                serial = "emulator-5554"
                output = Path(directory)
                commands = []

                def run(self, *args):
                    self.commands.append(args)
                    return {
                        ("shell", "getprop", "ro.kernel.qemu"): b"1\n",
                        ("shell", "wm", "size"): b"Physical size: 1080x2400\nOverride size: 800x1600\n",
                        ("shell", "wm", "density"): b"Physical density: 420\nOverride density: 300\n",
                    }.get(args, b"")
            device = Device()
            with patch.object(schedules.time, "sleep"), self.assertRaises(KeyboardInterrupt):
                with schedules.geometry(device, "small"):
                    raise KeyboardInterrupt()
            self.assertEqual(device.commands[-2:], [("shell", "wm", "density", "300"),
                                                   ("shell", "wm", "size", "800x1600")])
            self.assertTrue((Path(directory) / "display-before.json").is_file())

    def test_geometry_restores_even_if_second_override_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            class Device:
                serial = "emulator-5554"
                output = Path(directory)
                commands = []

                def run(self, *args):
                    self.commands.append(args)
                    if args == ("shell", "wm", "density", 320):
                        raise RuntimeError("injected display failure")
                    return b"1\n" if "getprop" in args else b"Physical size: 1080x2400\n"
            device = Device()
            with self.assertRaises(RuntimeError):
                with schedules.geometry(device, "small"):
                    pass
            self.assertEqual(device.commands[-2:], [("shell", "wm", "density", "reset"),
                                                   ("shell", "wm", "size", "reset")])


if __name__ == "__main__":
    unittest.main()

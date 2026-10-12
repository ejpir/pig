#!/usr/bin/env python3
"""Test gesture targeting without ADB; layout bounds can include clipped rows."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from test_interactions import Phone


ROW = "session-row-4"
SCROLL = 'scroll:Name("sessions")'


def state(row, scroll=(0, 112, 384, 630)):
    return {
        "viewport": [384, 854],
        "scale": 2,
        "bounds": {ROW: row, SCROLL: scroll},
    }


class GestureTargetTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.phone = Phone("not-a-device", Path(directory.name))

    def test_row_behind_dock_is_revealed_even_when_inside_window(self):
        clipped = state([20, 722, 344, 64])
        visible = state([20, 630, 344, 64])
        with patch.object(self.phone, "settled_state", side_effect=[clipped, visible]), \
                patch.object(self.phone, "swipe") as swipe:
            self.assertIs(self.phone.scroll_into_view(ROW, SCROLL), visible)
        swipe.assert_called_once()
        start, end = swipe.call_args.args
        self.assertEqual(start[0], end[0], "reveal with a vertical scroll, never a delete swipe")
        self.assertGreater(start[1], end[1])
        for x, y in (start, end):
            self.assertTrue(0 < x < 384 and 112 < y < 742, "stay inside the list, above the dock")

    def test_visible_center_is_not_enough_when_the_row_is_clipped(self):
        clipped = state([20, 700, 344, 64])
        visible = state([20, 640, 344, 64])
        with patch.object(self.phone, "settled_state", side_effect=[clipped, visible]), \
                patch.object(self.phone, "swipe") as swipe:
            self.assertIs(self.phone.scroll_into_view(ROW, SCROLL), visible)
        swipe.assert_called_once()

    def test_visible_row_needs_no_scroll(self):
        visible = state([20, 400, 344, 64])
        with patch.object(self.phone, "settled_state", return_value=visible), \
                patch.object(self.phone, "swipe") as swipe:
            self.assertIs(self.phone.scroll_into_view(ROW, SCROLL), visible)
        swipe.assert_not_called()

    def test_row_behind_header_scrolls_down(self):
        clipped = state([20, 90, 344, 64])
        visible = state([20, 160, 344, 64])
        with patch.object(self.phone, "settled_state", side_effect=[clipped, visible]), \
                patch.object(self.phone, "swipe") as swipe:
            self.assertIs(self.phone.scroll_into_view(ROW, SCROLL), visible)
        start, end = swipe.call_args.args
        self.assertLess(start[1], end[1])
        self.assertGreater(start[1], 112)
        self.assertLess(end[1], 742)

    def test_scroll_clip_is_intersected_with_the_window(self):
        clipped = state([20, 820, 344, 64], scroll=(0, 112, 384, 1000))
        visible = state([20, 700, 344, 64], scroll=(0, 112, 384, 1000))
        with patch.object(self.phone, "settled_state", side_effect=[clipped, visible]), \
                patch.object(self.phone, "swipe") as swipe:
            self.assertIs(self.phone.scroll_into_view(ROW, SCROLL), visible)
        for _, y in swipe.call_args.args:
            self.assertLess(y, 854)

    def test_impossible_targets_fail_without_injecting_input(self):
        for row in ([20, 100, 344, 700], [-20, 400, 344, 64], [20, 400, 400, 64],
                    [20, 400, 0, 64], [20, 400, 344, 0]):
            with self.subTest(row=row), \
                    patch.object(self.phone, "settled_state", return_value=state(row)), \
                    patch.object(self.phone, "swipe") as swipe:
                with self.assertRaises(AssertionError):
                    self.phone.scroll_into_view(ROW, SCROLL)
                swipe.assert_not_called()

    def test_unreachable_row_fails_instead_of_swiping_the_dock(self):
        with patch.object(self.phone, "settled_state", return_value=state([20, 722, 344, 64])), \
                patch.object(self.phone, "swipe") as swipe:
            with self.assertRaisesRegex(AssertionError, "did not scroll into"):
                self.phone.scroll_into_view(ROW, SCROLL)
            self.assertEqual(swipe.call_count, 8, "scroll attempts must be bounded")


if __name__ == "__main__":
    unittest.main()

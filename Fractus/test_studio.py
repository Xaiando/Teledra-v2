"""Playback and decode regressions; these tests never open a gallery window."""

from collections import OrderedDict
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image

from studio import FractusStudio, PlaybackClock, artwork_title, decode_artifact, fit_size


class PlaybackTests(unittest.TestCase):
    def test_elapsed_time_skips_late_frames_without_slowing_the_loop(self):
        clock = PlaybackClock([100] * 10, now=0.0)
        self.assertEqual(clock.frame_at(0.025), 0)
        self.assertEqual(clock.frame_at(0.475), 4)
        self.assertEqual(clock.frame_at(12.775), 7)
        self.assertAlmostEqual(clock.position(12.775), 0.775)

    def test_variable_durations_and_loop_boundary(self):
        clock = PlaybackClock([40, 80, 200], now=0.0)
        self.assertEqual(clock.frame_at(0.039), 0)
        self.assertEqual(clock.frame_at(0.04), 1)
        self.assertEqual(clock.frame_at(0.119), 1)
        self.assertEqual(clock.frame_at(0.12), 2)
        self.assertEqual(clock.frame_at(0.321), 0)

    def test_pause_resume_preserves_remaining_frame_time(self):
        clock = PlaybackClock([100] * 10, now=0.0)
        clock.set_playing(False, 0.175)
        self.assertEqual(clock.frame_at(45.0), 1)
        self.assertAlmostEqual(clock.position(45.0), 0.175)
        clock.set_playing(True, 45.0)
        self.assertEqual(clock.frame_at(45.020), 1)
        self.assertEqual(clock.frame_at(45.030), 2)

    def test_speed_changes_preserve_position(self):
        clock = PlaybackClock([100] * 10, now=0.0)
        clock.set_speed(0.5, 0.175)
        self.assertAlmostEqual(clock.position(0.175), 0.175)
        self.assertEqual(clock.frame_at(0.215), 1)
        self.assertEqual(clock.frame_at(0.235), 2)
        clock.set_playing(False, 0.235)
        clock.set_speed(1.5, 20.0)
        self.assertAlmostEqual(clock.position(21.0), 0.205)

    def test_frame_stepping_wraps_and_remains_paused(self):
        clock = PlaybackClock([40, 80, 200], now=0.0, playing=False)
        clock.seek_frame(-1, 0.0)
        self.assertEqual(clock.frame_at(400), 2)
        clock.seek_frame(3, 400)
        self.assertEqual(clock.frame_at(500), 0)

    def test_24fps_schedule_retains_fractional_timing(self):
        clock = PlaybackClock([1000 / 24] * 120, now=0.0)
        self.assertAlmostEqual(clock.duration, 5.0)
        self.assertEqual(clock.frame_at(4.99), 119)
        self.assertEqual(clock.frame_at(5.01), 0)
        self.assertEqual(clock.next_delay_ms(0.020), 22)
        clock.set_speed(1.5, 0.020)
        self.assertEqual(clock.next_delay_ms(0.020), 15)

    def test_invalid_timing_is_rejected(self):
        for durations in ([], [0], [-1], [float("nan")], [float("inf")]):
            with self.assertRaises(ValueError):
                PlaybackClock(durations, 0)
        for speed in (0, -1, float("nan")):
            with self.assertRaises(ValueError):
                PlaybackClock([100], 0, speed=speed)


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.directory = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def test_apng_preserves_true_colour_and_fractional_durations(self):
        first = Image.new("RGB", (32, 32))
        first.putdata([(x, y, (x + y) % 256) for y in range(32) for x in range(32)])
        second = Image.new("RGB", (32, 32), (9, 31, 57))
        path = self.directory / "motion.png"
        first.save(path, save_all=True, append_images=[second], duration=[41.667, 83.333], loop=0)
        frames, durations = decode_artifact(path)
        self.assertEqual(len(frames), 2)
        self.assertEqual(frames[0].mode, "RGBA")
        self.assertGreater(len(frames[0].getcolors(maxcolors=1024)), 256)
        self.assertEqual(frames[0].getpixel((17, 21)), (17, 21, 38, 255))
        self.assertEqual(frames[1].getpixel((17, 21)), (9, 31, 57, 255))
        self.assertAlmostEqual(durations[0], 41.667, places=2)
        self.assertAlmostEqual(durations[1], 83.333, places=2)

    def test_apng_poster_is_excluded_from_playback(self):
        path = self.directory / "poster.png"
        Image.new("RGB", (4, 4), "red").save(
            path, save_all=True, default_image=True, loop=0,
            append_images=[Image.new("RGB", (4, 4), "green"), Image.new("RGB", (4, 4), "blue")],
            duration=[40, 80],
        )
        frames, durations = decode_artifact(path)
        self.assertEqual(len(frames), 2)
        self.assertEqual(frames[0].getpixel((0, 0)), (0, 128, 0, 255))
        self.assertEqual(durations, [40, 80])

    def test_still_and_legacy_gif_remain_supported(self):
        still = self.directory / "still.png"
        Image.new("RGBA", (8, 12), (12, 34, 56, 78)).save(still)
        frames, _ = decode_artifact(still)
        self.assertEqual(len(frames), 1)
        self.assertEqual(frames[0].getpixel((2, 2)), (12, 34, 56, 78))
        animation = self.directory / "legacy.gif"
        Image.new("RGB", (8, 12), "red").save(
            animation, save_all=True, append_images=[Image.new("RGB", (8, 12), "blue")],
            duration=[70, 140], loop=0,
        )
        frames, durations = decode_artifact(animation)
        self.assertEqual(len(frames), 2)
        self.assertEqual(durations, [70, 140])

    def test_excess_frame_count_is_rejected_instead_of_truncated(self):
        path = self.directory / "too-many.png"
        Image.new("RGB", (2, 2), "red").save(
            path, save_all=True, append_images=[Image.new("RGB", (2, 2), "blue")], duration=40,
        )
        with patch("studio.MAX_FRAMES", 1), self.assertRaisesRegex(ValueError, "limit of 1 frames"):
            decode_artifact(path)

    def test_decoded_pixel_limit_is_checked(self):
        path = self.directory / "too-large.png"
        Image.new("RGB", (8, 8), "red").save(path)
        with patch("studio.MAX_DECODED_PIXELS", 32), self.assertRaisesRegex(ValueError, "decoded-frame"):
            decode_artifact(path)

    def test_artifact_path_cannot_escape_trusted_root(self):
        path = self.directory / "outside.png"
        Image.new("RGB", (2, 2), "red").save(path)
        studio = FractusStudio.__new__(FractusStudio)
        studio.trusted_root = (self.directory / "trusted").resolve()
        with self.assertRaisesRegex(ValueError, "outside the trusted"):
            studio.validate_path(path)

    def test_artwork_title_prefers_bounded_printable_command_name(self):
        path = self.directory / "fractus-1234567890.png"
        self.assertEqual(artwork_title(path, "\n Lotus\x00 Study\t"), "Lotus Study")
        self.assertEqual(len(artwork_title(path, "A" * 1000)), 100)

    def test_artwork_title_uses_companion_scene_and_palette(self):
        path = self.directory / "fractus-1234567890.png"
        path.with_suffix(".manifest.json").write_text(json.dumps({
            "scene": {"kind": "lotus_mandala", "palette": "electric_cyan"}
        }), encoding="utf-8")
        self.assertEqual(artwork_title(path), "Lotus Mandala · Electric Cyan")
        self.assertEqual(artwork_title(path, "Sunlit Petals"), "Sunlit Petals")

    def test_artwork_title_humanizes_names_and_hides_timestamp_ids(self):
        self.assertEqual(artwork_title(self.directory / "01-lotus-observatory.png"), "Lotus Observatory")
        self.assertEqual(artwork_title(self.directory / "fractus-1748879012932-15.png"), "Geometric study")
        self.assertEqual(artwork_title(self.directory / "fractus-2026-09-13T10-15-00Z.png"), "Geometric study")

    def test_invalid_and_oversized_metadata_fall_back_to_display_name(self):
        path = self.directory / "lotus-study.png"
        manifest = path.with_suffix(".manifest.json")
        for content in ("{invalid", "[]", '{"scene": "unexpected"}', "[" * 2000 + "0" + "]" * 2000):
            manifest.write_text(content, encoding="utf-8")
            self.assertEqual(artwork_title(path), "Lotus Study")
        manifest.write_text('{"scene":{"kind":"julia"}}', encoding="utf-8")
        with patch("studio.MAX_MANIFEST_BYTES", 8):
            self.assertEqual(artwork_title(path), "Lotus Study")


class PresentationTests(unittest.TestCase):
    def test_portrait_landscape_and_small_viewports_fit_without_crop(self):
        self.assertEqual(fit_size((600, 800), (1200, 800)), (600, 800))
        self.assertEqual(fit_size((600, 800), (300, 500)), (300, 400))
        self.assertEqual(fit_size((800, 400), (600, 900)), (600, 300))
        self.assertEqual(fit_size((600, 800), (0, 0)), (1, 1))

    def test_enormous_viewports_cannot_allocate_unbounded_render_images(self):
        with patch("studio.MAX_RENDER_PIXELS", 800):
            width, height = fit_size((600, 800), (100_000, 100_000))
            self.assertLessEqual(width * height, 800)
            self.assertAlmostEqual(width / height, 0.75, delta=0.03)

    def test_resize_cache_is_bounded_and_unchanged_frame_is_reused(self):
        studio = FractusStudio.__new__(FractusStudio)
        studio.source_frames = [Image.new("RGBA", (2, 2), color) for color in ("red", "green", "blue")]
        studio.playing = True
        studio.frame_index = 0
        studio.render_cache = OrderedDict()
        studio.cache_pixels = 0
        with patch("studio.MAX_CACHE_PIXELS", 32):
            first = studio.render_frame((4, 4))
            self.assertIs(studio.render_frame((4, 4)), first)
            studio.frame_index = 1
            studio.render_frame((4, 4))
            studio.frame_index = 2
            studio.render_frame((4, 4))
            self.assertEqual(studio.cache_pixels, 32)
            self.assertEqual(len(studio.render_cache), 2)
            studio.render_frame((8, 8))
            self.assertEqual(studio.cache_pixels, 0)
            self.assertEqual(len(studio.render_cache), 0)


if __name__ == "__main__":
    unittest.main()

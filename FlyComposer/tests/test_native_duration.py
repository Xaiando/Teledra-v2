"""Note lengths the native patch renderer and the Python renderer agree on."""
import random
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

import numpy as np

from flycomposer.render import NATIVE_RATE, native_duration


class NativeDurationTests(unittest.TestCase):
    def test_the_length_that_crashed_a_dusk_render_lands_mid_frame(self):
        # 0.986 s is stored as 0.9859999999999999. Python truncates that to 47327
        # frames; 32-bit arithmetic on the native side makes exactly 47328.
        crashed = 0.9859999999999999
        self.assertEqual(int(crashed * NATIVE_RATE), 47327)
        self.assertEqual(int(np.float32(crashed) * np.float32(NATIVE_RATE)), 47328)
        fixed = native_duration(crashed, NATIVE_RATE)
        python_frames = int(fixed * NATIVE_RATE)
        native_frames = int(np.float32(fixed) * np.float32(NATIVE_RATE))
        self.assertEqual(python_frames, native_frames)
        self.assertEqual(python_frames, 47327)

    def test_a_length_safely_inside_a_frame_is_left_exactly_alone(self):
        # Every render that worked before must stay byte-identical.
        for seconds in (.5 + .3 / NATIVE_RATE, 1.25 + .4 / NATIVE_RATE, 12.7 + .6 / NATIVE_RATE):
            self.assertEqual(native_duration(seconds, NATIVE_RATE), seconds)

    def test_the_frame_count_at_the_target_rate_never_changes(self):
        rng = random.Random(20260917)
        for sr in (48000, 44100, 16000):
            for _ in range(20000):
                seconds = rng.uniform(.01, 30.)
                self.assertEqual(int(native_duration(seconds, sr) * sr), int(seconds * sr))

    def test_32_and_64_bit_arithmetic_agree_on_every_boundary_length(self):
        # The risky lengths are the ones a hair under a whole frame.
        for frames in range(1000, 200000, 997):
            seconds = np.nextafter(frames / NATIVE_RATE, 0.)
            fixed = native_duration(float(seconds), NATIVE_RATE)
            self.assertEqual(int(fixed * NATIVE_RATE),
                             int(np.float32(fixed) * np.float32(NATIVE_RATE)), frames)


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import numpy as np

from music_verify import verify_file


GOOD = """
import numpy as np
from teledra_synth import *
sr = 8000
t = np.arange(sr, dtype=float) / sr
lead = 0.2 * np.sin(2 * np.pi * 220 * t)
bass = 0.1 * np.sin(2 * np.pi * 110 * t)
full_track = lead + bass
TELEDRA_LAYERS = {"lead": lead, "bass": bass}
play_sound(full_track, sr=sr, loop=True)
"""

TYPO = """
import numpy as np
from teledra_synth import *
sr = 8000
lead = synth_note("H4", 1.0, sr=sr)
pad = synth_note("C4", 1.0, sr=sr, volume=0.1)
full_track = lead + pad
TELEDRA_LAYERS = {"lead": lead, "pad": pad}
play_sound(full_track, sr=sr, loop=True)
"""

DEAD = """
import numpy as np
from teledra_synth import *
sr = 8000
t = np.arange(sr, dtype=float) / sr
lead = 0.2 * np.sin(2 * np.pi * 220 * t)
counterline = np.zeros_like(lead)
full_track = lead + counterline
TELEDRA_LAYERS = {"lead": lead, "counterline": counterline}
play_sound(full_track, sr=sr, loop=True)
"""

CLIPPING = """
import numpy as np
from teledra_synth import *
sr = 8000
t = np.arange(sr, dtype=float) / sr
lead = 0.8 * np.sin(2 * np.pi * 220 * t)
bass = 0.8 * np.sin(2 * np.pi * 220 * t)
full_track = lead + bass
TELEDRA_LAYERS = {"lead": lead, "bass": bass}
play_sound(full_track, sr=sr, loop=True)
"""


class MusicVerifierTests(unittest.TestCase):
    def verify(self, source: str):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "candidate.py"
            path.write_text(source, encoding="utf-8")
            return verify_file(path)

    def test_good_track_passes(self):
        report = self.verify(GOOD)
        self.assertTrue(report["ok"], report)

    def test_typoed_note_reports_invalid_note(self):
        report = self.verify(TYPO)
        self.assertFalse(report["ok"])
        self.assertIn("invalid_note", {issue["code"] for issue in report["issues"]})

    def test_dead_layer_reports_layer_name(self):
        report = self.verify(DEAD)
        self.assertFalse(report["ok"])
        issue = next(issue for issue in report["issues"] if issue["code"] == "dead_layer")
        self.assertEqual("counterline", issue["layer"])

    def test_clipping_reports_peak(self):
        report = self.verify(CLIPPING)
        self.assertFalse(report["ok"])
        issue = next(issue for issue in report["issues"] if issue["code"] == "clipping")
        self.assertGreaterEqual(issue["peak"], issue["threshold"])


if __name__ == "__main__":
    unittest.main()

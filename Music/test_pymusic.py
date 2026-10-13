import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import wave

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import teledra_synth as synth
from python_music_worker import render_source, validate_source, _stereo_48k


FREEFORM = '''
import numpy as np
from teledra_synth import synth_note, mix_waves, stereo_pan, play_sound
sr = 8000
TELEDRA_SCORE = {"title": "A free Python answer", "bpm": 90}
full_track = np.zeros((sr * 4, 2))
for index, note in enumerate(["C4", "E4", "G4", "E4", "D4", "G4", "C4"]):
    voice = synth_note(note, .8, attack=.01, release=.15, volume=.12, sr=sr)
    full_track = mix_waves(full_track, stereo_pan(voice, -.3 if index % 2 else .3),
                           start_time=index * .45, sr=sr)
play_sound(full_track, sr=sr, loop=False)
'''


class PythonMusicTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("TELEDRA_SYNTH_EXE"), "requires built native synth")
    def test_dispatch_contract_example_is_complete_and_executable(self):
        guide = HERE.joinpath("PYTHON_AUTHORING.md").read_text(encoding="utf-8")
        code = guide.split("```python\n", 1)[1].split("```", 1)[0]
        audio, report = render_source(code, synth_exe=os.environ["TELEDRA_SYNTH_EXE"])
        self.assertEqual(report["notes_requested"], 10)
        self.assertEqual(report["notes_rendered"], 10)
        self.assertEqual(report["duration_seconds"], 8)
        for section in np.array_split(audio, 8):
            self.assertGreater(np.sqrt(np.mean(section ** 2)), .0001)

    def tearDown(self):
        synth._capture_callback = None
        synth._close_bridge()

    def test_import_and_unowned_play_sound_never_open_a_device(self):
        source = HERE.joinpath("teledra_synth.py").read_text(encoding="utf-8")
        self.assertNotIn("import sounddevice", source)
        self.assertNotIn("import tkinter", source)
        with self.assertRaisesRegex(RuntimeError, "worker"):
            synth.play_sound(np.ones(10), sr=8000)

    def test_v1_waveform_pitch_envelope_and_pink_noise_contract(self):
        self.assertAlmostEqual(synth.note_to_freq("A4"), 440)
        self.assertAlmostEqual(synth.note_to_freq("eb3"), synth.note_to_freq("D#3"))
        self.assertEqual(synth.note_to_freq(220), 220)
        with self.assertRaises(ValueError):
            synth.note_to_freq("H4")
        short = synth.synth_note("C4", .02, attack=.3, decay=.1, release=.2, sr=8000)
        self.assertEqual(len(short), 160)
        self.assertTrue(np.all(np.isfinite(short)))
        self.assertGreater(np.max(np.abs(short)), .04)
        np.testing.assert_array_equal(synth.pink_noise(.2, sr=8000, seed=5),
                                      synth.pink_noise(.2, sr=8000, seed=5))

    def test_note_overlays_extend_and_sum_every_overlapping_sample(self):
        a = np.zeros(12)
        first = np.arange(1, 9, dtype=float)
        second = np.ones(8) * .5
        result = synth.mix_waves(a, first, start_time=.004, sr=1000)
        result = synth.mix_waves(result, second, start_time=.008, sr=1000)
        self.assertEqual(len(result), 16)
        np.testing.assert_array_equal(result[4:8], first[:4])
        np.testing.assert_array_equal(result[8:12], first[4:] + second[:4])
        np.testing.assert_array_equal(result[12:], second[4:])

    def test_freeform_piece_is_resampled_and_audited_without_grid_rewriting(self):
        audio, report = render_source(FREEFORM)
        self.assertEqual(audio.shape, (192000, 2))
        self.assertEqual(report["title"], "A free Python answer")
        self.assertEqual((report["notes_requested"], report["notes_rendered"]), (7, 7))
        self.assertFalse(report["loop"])
        self.assertGreater(report["rms"], .01)
        self.assertLess(report["peak"], .95)
        self.assertFalse(np.array_equal(audio[:, 0], audio[:, 1]))

    def test_numpy_only_composition_does_not_invent_note_counts(self):
        source = '''import numpy as np
from teledra_synth import play_sound
wave = np.sin(2 * np.pi * 220 * np.arange(32000) / 8000) * .05
play_sound(wave, sr=8000)
'''
        _, report = render_source(source)
        self.assertEqual(report["notes_requested"], 0)
        self.assertIn("arbitrary NumPy", report["note_audit_scope"])

    def test_resampling_preserves_timing_past_windows_integer_overflow_and_loop_seam(self):
        sr = 44100
        t = np.arange(sr * 4, dtype=np.float64) / sr
        original = synth.make_seamless_loop(np.sin(2 * np.pi * 220 * t) * .1, sr=sr)
        result = _stereo_48k(original, sr)
        positions = np.arange(48000 * 4, dtype=np.float64) * (sr / 48000.0)
        expected = np.interp(positions, np.arange(len(original)), original)
        np.testing.assert_allclose(result[:, 0], expected, atol=1e-12)
        np.testing.assert_array_equal(result[:, 0], result[:, 1])
        self.assertAlmostEqual(result[0, 0], result[-1, 0], places=12)

    def test_caught_note_failures_cannot_publish_an_incomplete_composition(self):
        source = '''from teledra_synth import synth_note, play_sound
try:
    synth_note("H4", .5)
except ValueError:
    pass
play_sound(np.ones(32000)*.1, sr=8000)
'''
        with self.assertRaisesRegex(ValueError, "incomplete note synthesis"):
            render_source(source)

    def test_main_guard_and_explicit_rest_remain_valid_python(self):
        source = '''from teledra_synth import synth_note, mix_waves, play_sound
if __name__ == "__main__":
    rest = synth_note(None, 4, sr=8000)
    tone = synth_note("C4", 4, sr=8000)
    play_sound(mix_waves(rest, tone, sr=8000), sr=8000)
'''
        _, report = render_source(source)
        self.assertEqual((report["notes_requested"], report["notes_rendered"]), (2, 2))

    def test_unsafe_imports_private_access_and_numpy_file_helpers_are_rejected(self):
        for source in ["import os", "import subprocess", "from numpy import load", "np.save('x', [1])",
                       "np.load('x')", "np.ctypeslib.load_library('x', '.')",
                       "synth_note.__globals__", "'{0.__globals__}'.format(synth_note)",
                       "open('x', 'w')", "eval('1+1')"]:
            with self.subTest(source=source), self.assertRaises((ValueError, ImportError, NameError)):
                render_source(source)

    def test_truncated_missing_multiple_silent_and_invalid_outputs_fail(self):
        cases = [
            "full_track = [", "full_track = [0, 1]",
            "from teledra_synth import play_sound\nplay_sound(np.ones(32000)*.1,sr=8000)\nplay_sound(np.ones(32000)*.1,sr=8000)",
            "from teledra_synth import play_sound\nplay_sound(np.zeros(32000),sr=8000)",
            "from teledra_synth import play_sound\nplay_sound(np.ones(100)*.1,sr=8000)",
            "from teledra_synth import play_sound\nplay_sound(np.ones(32000)*float('nan'),sr=8000)",
            "np.zeros(1000000000000)",
        ]
        for source in cases:
            with self.subTest(source=source), self.assertRaises((ValueError, SyntaxError)):
                render_source(source)

    def test_worker_writes_complete_stereo_wav_and_machine_readable_report(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            code, output, report_path = (directory / name for name in ("music.py", "music.wav", "report.json"))
            code.write_text(FREEFORM, encoding="utf-8")
            result = subprocess.run([sys.executable, str(HERE / "python_music_worker.py"),
                                     "--code", str(code), "--output", str(output), "--report", str(report_path)],
                                    capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            report = json.loads(report_path.read_text())
            self.assertTrue(report["ok"])
            with wave.open(str(output)) as wav:
                self.assertEqual((wav.getnchannels(), wav.getframerate(), wav.getnframes()), (2, 48000, 192000))
                self.assertEqual(wav.getsampwidth(), 2)

    @unittest.skipUnless(os.environ.get("TELEDRA_SYNTH_EXE"), "requires the built native note server")
    def test_every_native_instrument_returns_complete_short_notes_and_overlaps(self):
        synth._configure_bridge(os.environ["TELEDRA_SYNTH_EXE"])
        catalog = synth.instruments()
        self.assertGreaterEqual(len(catalog), 70)
        self.assertIn("piano", synth.INSTRUMENTS)
        self.assertIn("theremin", synth.INSTRUMENTS)
        for entry in catalog:
            with self.subTest(instrument=entry["id"]):
                low, high = entry["range"]
                hz = 440 * 2 ** (((low + high) / 2 - 69) / 12)
                voice = synth.instrument_note(hz, .04, entry["id"], volume=.5, sr=44100)
                self.assertEqual(len(voice), 1764)
                self.assertTrue(np.all(np.isfinite(voice)))
                self.assertGreater(np.max(np.abs(voice)), .001)
                repeated = synth.instrument_note(hz, .04, entry["id"], volume=.5, sr=44100)
                np.testing.assert_array_equal(voice, repeated)
                overlaid = synth.mix_waves(voice, repeated, .02, sr=44100)
                self.assertEqual(len(overlaid), 2646)
                np.testing.assert_allclose(overlaid[882:1764], voice[882:] + repeated[:882])
        with self.assertRaisesRegex(ValueError, "Nyquist"):
            synth.instrument_note(2637, .1, "glockenspiel", sr=4000)
        # A downsampled native note must suppress upper harmonics before they
        # can fold. Compare its low-rate output to the exact bandlimited source.
        native = synth.instrument_note(440, .2, "piano", sr=48000)
        reduced = synth.instrument_note(440, .2, "piano", sr=8000)
        spectrum = np.fft.rfft(native)
        spectrum[np.fft.rfftfreq(len(native), 1 / 48000) >= 4000] = 0
        expected = np.fft.irfft(spectrum, n=len(native))[::6]
        np.testing.assert_allclose(reduced, expected, atol=1e-8)


if __name__ == "__main__":
    unittest.main()

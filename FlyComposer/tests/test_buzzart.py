"""Buzzart's brain, performer, critic and trainer."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from flycomposer.autonomous import write_performance
from flycomposer.buzzart import critic
from flycomposer.buzzart.brain import SCALAR_BOUNDS, Brain
from flycomposer.buzzart.performer import perform
from flycomposer.buzzart.trainer import Trainer, centered_ranks
from flycomposer.connectome import load_circuit
from flycomposer.render import assess_performance


class BuzzartTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.circuit = load_circuit()

    def test_untrained_brain_without_context_reproduces_fly_exactly(self):
        for seed in (0, 2026):
            brain = Brain.from_controller(self.circuit, seed, context_scale=0)
            original = write_performance(self.circuit, seed, 40)
            buzzart = perform(self.circuit, brain, seed, 40)
            self.assertEqual(buzzart["events"], original["events"])
            self.assertEqual(buzzart["neural_trace"], original["neural_trace"])
            self.assertEqual(buzzart["performer"], "buzzart")
            assess_performance(buzzart)

    def test_performer_restores_the_fly_controller(self):
        import flycomposer.autonomous as autonomous
        from flycomposer.controller import Controller
        perform(self.circuit, Brain.from_controller(self.circuit), 1, 15)
        self.assertIs(autonomous.Controller, Controller)

    def test_context_gives_one_brain_different_music_per_seed(self):
        brain = Brain.from_controller(self.circuit)
        scores = [perform(self.circuit, brain, seed, 40) for seed in (11, 12, 13)]
        self.assertEqual(len({s["event_sha256"] for s in scores}), 3)
        silent = Brain.from_controller(self.circuit, context_scale=0)
        same = [perform(self.circuit, silent, seed, 40) for seed in (11, 12)]
        self.assertLess(critic.diversity(same)[0], critic.diversity(scores)[0])

    def test_brain_file_roundtrip_and_tamper_detection(self):
        brain = Brain.from_controller(self.circuit)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "brain.npz"
            brain.save(path, note="roundtrip")
            loaded = Brain.load(path)
            self.assertEqual(loaded.sha256(), brain.sha256())
            self.assertEqual(loaded.metadata["note"], "roundtrip")
            data = bytearray(path.read_bytes())
            data[-40] ^= 1
            path.write_bytes(bytes(data))
            with self.assertRaises(ValueError):
                Brain.load(path)

    def test_parameter_vector_keeps_scalars_in_bounds(self):
        brain = Brain.from_controller(self.circuit)
        # Scalars pass through a logit and back, which may move them by one float step.
        np.testing.assert_allclose(brain.with_vector(brain.vector()).vector(), brain.vector(), rtol=0, atol=1e-12)
        for extreme in (-1e6, 1e6):
            vector = brain.vector()
            vector[brain.group_slices()["scalars"]] = extreme
            moved = brain.with_vector(vector)
            for name, (low, high) in SCALAR_BOUNDS.items():
                self.assertTrue(low <= getattr(moved, name) <= high)

    def test_critic_is_bounded_and_prefers_melody_over_one_repeated_pitch(self):
        score = write_performance(self.circuit, 2026, 75)
        result = critic.evaluate(score)
        self.assertTrue(0 <= result["reward"] <= 1)
        self.assertEqual(set(result["components"]), set(critic.COMPONENTS))
        flat = copy.deepcopy(score)
        for event in flat["events"]:
            if event["layer"] in ("lead", "answer"):
                event["midi"] = 72
        self.assertLess(critic.evaluate(flat)["components"]["melody"], result["components"]["melody"])
        self.assertLess(critic.diversity([score, copy.deepcopy(score)])[0],
                        critic.diversity([score, write_performance(self.circuit, 7, 75)])[0])

    def test_band_shape(self):
        self.assertEqual(critic.band(.5, .4, .6, .2), 1.)
        self.assertAlmostEqual(critic.band(.7, .4, .6, .2), .5)
        self.assertEqual(critic.band(float("nan"), 0, 1, 1), 0.)
        np.testing.assert_allclose(centered_ranks([3., 1., 2.]), [.5, -.5, 0.])

    def test_one_training_generation_moves_the_brain_and_records_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory) / "run"
            logs = []
            trainer = Trainer(ROOT, run, Brain.from_controller(self.circuit), population=4, seeds_per_candidate=2,
                              seconds=20, workers=1, validation_seeds=[90_000, 90_001], validation_group=2, log=logs.append)
            start = trainer.theta.copy()
            trainer.run((("foundation", 1),), validate_every=1)
            self.assertEqual(trainer.generation, 1)
            self.assertFalse(np.array_equal(trainer.theta, start))
            history = [json.loads(line) for line in (run / "history.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertEqual(history[0]["generation"], 0)
            self.assertIn("validation", history[1])
            for name in ("latest.npz", "best.npz", "state.json", "optimizer.npz"):
                self.assertTrue((run / name).exists(), name)
            resumed = Trainer(ROOT, run, workers=1, validation_seeds=[90_000, 90_001], validation_group=2, log=logs.append)
            np.testing.assert_array_equal(resumed.theta, trainer.theta)
            self.assertEqual(resumed.generation, 1)


SYNTHETIC_SONGBOOK = {
    "intervals": {"-3": .06, "-2": .14, "-1": .24, "0": .12, "1": .24, "2": .12, "3": .05, "4": .03},
    "interval_bigrams": {"1": {"1": 5, "-1": 3, "2": 2}, "-1": {"-1": 4, "1": 3, "0": 1}, "0": {"1": 2, "-1": 2}},
    "steps": {".5": .45, "1": .35, "1.5": .12, "2": .08},
    "rests": {"0": .8, ".5": .15, "1": .05},
    "iois": {".5": .4, "1": .4, "1.5": .1, "2": .1},
    "root_transitions": {"major": {"0": {"3": .4, "4": .5, "5": .1}, "4": {"0": .8, "5": .2}, "3": {"4": .6, "0": .4}, "5": {"3": .5, "4": .5}},
                         "minor": {"0": {"3": .3, "4": .4, "5": .3}, "4": {"0": .9, "5": .1}, "3": {"4": .7, "0": .3}, "5": {"6": .5, "4": .5}, "6": {"0": 1.}}},
    "motifs": [{"id": "test:a", "family": "major", "degrees": [0, 2, 4, 2, 0], "steps": [1, .5, .5, 1, 1]},
               {"id": "test:b", "family": "major", "degrees": [4, 3, 2, 1, 0], "steps": [.5, .5, .5, .5, 1]},
               {"id": "test:c", "family": "minor", "degrees": [0, -1, 0, 2, 4], "steps": [1, 1, .5, .5, 1]},
               {"id": "test:d", "family": "minor", "degrees": [4, 2, 1, 0, -1], "steps": [.5, 1, .5, 1, 1]},
               {"id": "test:e", "family": "major", "degrees": [2, 4, 5, 4, 2], "steps": [1, 1, 1, 1, 1]}],
    "reference": {"interval_logprob": -1.8, "ioi_logprob": -1.2, "root_logprob": -1.1},
}


class FoundationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from flycomposer.buzzart.foundation import Foundation
        cls.circuit = load_circuit()
        cls.foundation = Foundation(SYNTHETIC_SONGBOOK)

    def test_foundation_performance_is_valid_repeatable_and_opens_with_songbook_motifs(self):
        brain = Brain.from_controller(self.circuit)
        score = perform(self.circuit, brain, 90_003, 40, foundation=self.foundation)
        assess_performance(score)
        self.assertEqual(score["event_sha256"], perform(self.circuit, brain, 90_003, 40, foundation=self.foundation)["event_sha256"])
        self.assertNotEqual(score["event_sha256"], perform(self.circuit, brain, 90_003, 40)["event_sha256"])
        self.assertTrue(score["buzzart"]["opening_motifs"])
        self.assertTrue(all(source.startswith("test:") for source in score["buzzart"]["opening_motifs"]))
        self.assertEqual(score["buzzart"]["foundation"]["songbook_sha256"], self.foundation.sha256)

    def test_choices_follow_the_prior_and_the_drive(self):
        self.assertIn(self.foundation.interval(0.), (-1, 0, 1))
        self.assertGreater(self.foundation.interval(1.), 0)
        self.assertLess(self.foundation.interval(-1.), 0)
        self.assertEqual(self.foundation.rest(-1.), 0.)
        self.assertGreater(self.foundation.root_bias(4, 0, "major"), self.foundation.root_bias(4, 6, "major"))

    def test_a_performance_keeps_songbook_proportions_whatever_range_its_drives_use(self):
        from collections import Counter
        rng = np.random.default_rng(4)
        for drives in (rng.uniform(-1, 1, 1200), np.tanh(3 * rng.standard_normal(1200)), .3 + .02 * rng.standard_normal(1200)):
            choices = self.foundation.session()
            steps = Counter(choices.step(drive) for drive in drives)
            for value, share in SYNTHETIC_SONGBOOK["steps"].items():
                self.assertAlmostEqual(steps[float(value)] / len(drives), share, delta=.05)
            # The drive still decides: above everything recent is the longest length, below it the shortest.
            self.assertEqual(choices.step(5.), 2.)
            self.assertEqual(choices.step(-5.), .5)

    def test_ranked_openings_vary_key_and_tempo_across_seeds(self):
        brain = Brain.from_controller(self.circuit)
        scores = [perform(self.circuit, brain, seed, 12, foundation=self.foundation) for seed in range(10_000, 10_012)]
        self.assertGreaterEqual(len({s["tonic"] for s in scores}), 3)
        self.assertGreaterEqual(len({s["bpm"] for s in scores}), 6)
        self.assertTrue(all(0 < share < 1 for s in scores for share in s["buzzart"]["opening_shares"]))
        self.assertIsNone(perform(self.circuit, brain, 10_000, 12)["buzzart"]["opening_shares"])

    def test_fit_is_bounded_and_prefers_common_motion(self):
        score = perform(self.circuit, Brain.from_controller(self.circuit), 90_003, 40, foundation=self.foundation)
        result = self.foundation.fit(score)
        self.assertTrue(0 <= result["fit"] <= 1)
        self.assertEqual(set(result["parts"]), {"intervals", "rhythm", "harmony"})
        smooth = self.foundation.interval_logprobs([1, 1, -1, -1, 1, 1])
        jagged = self.foundation.interval_logprobs([7, -7, 6, -6, 7, -7])
        self.assertGreater(np.mean(smooth), np.mean(jagged))

    def test_tampered_songbook_is_refused(self):
        from flycomposer.buzzart.foundation import Foundation
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "songbook.json"
            raw = json.dumps(SYNTHETIC_SONGBOOK).encode()
            path.write_bytes(raw)
            import hashlib
            (Path(directory) / "songbook.manifest.json").write_text(json.dumps({"sha256": hashlib.sha256(raw).hexdigest()}))
            self.assertEqual(Foundation.load(path).sha256, hashlib.sha256(raw).hexdigest())
            path.write_bytes(raw.replace(b"test:a", b"test:z"))
            with self.assertRaises(ValueError):
                Foundation.load(path)


class ConsoleVoiceTests(unittest.TestCase):
    def test_every_console_voice_returns_exact_audible_frames(self):
        from flycomposer.buzzart import chip
        for voice in chip.CHIP_VOICES:
            for midi in (40, 64, 80):
                event = {"layer": "lead", "midi": midi, "start_beat": 2., "duration_seconds": .37}
                sound = chip.render_voice(voice, event, 48000, seed=3)
                self.assertEqual(len(sound), int(.37 * 48000), voice)
                self.assertTrue(np.all(np.isfinite(sound)))
                self.assertGreater(float(np.sqrt(np.mean(sound ** 2))), 1e-3, voice)

    def test_palettes_render_through_the_fly_renderer(self):
        from flycomposer.buzzart import chip
        from flycomposer.render import render_score
        circuit = load_circuit()
        for name, palette in chip.PALETTES.items():
            score = perform(circuit, Brain.from_controller(circuit), 5, 15, palette)
            proxy = copy.deepcopy(score)
            proxy["voices"] = {role: "classic:sine" for role in proxy["voices"]}
            for event in proxy["events"]:
                event["voice"] = "classic:sine"
            proxy["event_sha256"] = __import__("hashlib").sha256(json.dumps(proxy["events"], sort_keys=True).encode()).hexdigest()
            # A palette whose parts fall through the chain to PyMusic needs the
            # bridge open; the chain leaves its lifetime to the caller.
            from flycomposer.render import ROOT as RENDER_ROOT
            from flycomposer.vendor import teledra_synth as synth
            pymusic = chip.pymusic_voices(score["voices"])
            if pymusic:
                synth._configure_bridge(RENDER_ROOT / "bin/PyMusicSynth.exe")
            try:
                audio, stems, audit = render_score(proxy, chip.note_factory(score["voices"], score["seed"], score), 16000)
            finally:
                if pymusic:
                    synth._close_bridge()
            self.assertEqual(audit["notes_mixed"], len(score["events"]), name)
            self.assertGreater(audit["raw_rms"], 0, name)
            # Every part that fell through the chain actually made sound.
            for role in pymusic:
                if any(e["layer"] == role for e in score["events"]):
                    self.assertGreater(float(np.abs(stems[role]).max()), 0, f"{name}: {role} reached PyMusic silently")


if __name__ == "__main__":
    unittest.main()

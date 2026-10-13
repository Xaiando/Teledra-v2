"""Technical and provenance checks for a circuit-directed finite performance.

There is deliberately no section-count, preferred form or listening-quality gate.
"""
import hashlib
import json
import math
import copy
from pathlib import Path
import sys
import unittest
import tempfile
import wave
from unittest.mock import patch

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from flycomposer.autonomous import VOICE_POOLS, extend_performance, write_performance
from flycomposer.composition import ENSEMBLES, ROLES, SCALES, midi
from flycomposer.connectome import load_circuit
from flycomposer.controller import Controller, fingerprint
from flycomposer.render import render_score, save_wav


class AutonomousTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.circuit = load_circuit()

    def assert_technical_capture(self, score, duration):
        self.assertEqual(score["kind"], "performance")
        self.assertEqual(score["performer"], "fly")
        self.assertEqual(score["duration_seconds"], duration)
        self.assertEqual(set(score["voices"]), set(ROLES))
        self.assertTrue(score["events"])
        self.assertEqual([e["id"] for e in score["events"]], list(range(len(score["events"]))))
        self.assertEqual(score["event_sha256"], hashlib.sha256(json.dumps(score["events"], sort_keys=True).encode()).hexdigest())
        for event in score["events"]:
            self.assertTrue(all(math.isfinite(event[key]) for key in ("start_seconds", "duration_seconds", "velocity")))
            self.assertIn(event["layer"], ROLES)
            self.assertEqual(event["voice"], score["voices"][event["layer"]])
            self.assertGreater(event["duration_seconds"], 0)
            self.assertGreaterEqual(event["start_seconds"], 0)
            self.assertLessEqual(event["start_seconds"] + event["duration_seconds"], duration - 1 + 1e-8)
            self.assertTrue(0 <= event["midi"] <= 127)
            self.assertAlmostEqual(event["start_seconds"], event["start_beat"] * 60 / score["bpm"], places=5)
            self.assertAlmostEqual(event["duration_seconds"], event["length_beats"] * 60 / score["bpm"], places=5)
        for role in ("lead", "answer"):
            line = [e for e in score["events"] if e["layer"] == role]
            for a, b in zip(line, line[1:]):
                self.assertLessEqual(a["start_seconds"] + a["duration_seconds"], b["start_seconds"] + 1e-8)
        self.assertEqual(score["sections"][0]["start_seconds"], 0)
        self.assertEqual(score["sections"][0]["end_seconds"], duration - 1)
        self.assertIsNone(score["decoder_provenance"]["planned_form"])
        self.assertIsNone(score["decoder_provenance"]["planned_progression"])
        self.assertIsNone(score["decoder_provenance"]["prewritten_melody"])

    def test_capture_bounds_and_exact_ledger_without_old_arrangers(self):
        with patch("flycomposer.composition.write_song", side_effect=AssertionError("Old arranger used")), \
             patch("flycomposer.development.write_development", side_effect=AssertionError("Development grammar used")):
            for seed, duration in ((0, 10), (42, 37.25), (2026, 75), (2**31 - 1, 180)):
                with self.subTest(seed=seed, duration=duration):
                    score = write_performance(self.circuit, seed, duration)
                    self.assert_technical_capture(score, duration)
                    self.assertEqual(score["event_sha256"], write_performance(self.circuit, seed, duration)["event_sha256"])

    def test_decision_trace_replays_one_persistent_real_controller(self):
        score = write_performance(self.circuit, 2026)
        replay = Controller(self.circuit, 2026)
        self.assertEqual(len(score["decision_log"]), len(score["neural_trace"]))
        self.assertGreater(len(score["decision_log"]), 3)
        for decision, trace in zip(score["decision_log"], score["neural_trace"]):
            actual = replay.phrase(decision["index"], decision["input_feedback_energy"], decision["input_harmonic_root"])
            np.testing.assert_array_equal(actual, decision["outputs"])
            np.testing.assert_array_equal(actual, trace["outputs"])
            self.assertEqual(decision["selected_root"], int(np.argmax(decision["harmonic_scores"])))
            self.assertEqual(decision["selected_span_beats"], decision["span_candidates"][int(np.argmax(decision["span_scores"]))])
            sounded = [e for e in score["events"] if e["decision"] == decision["index"]]
            self.assertEqual(decision["notes_scheduled"], len(sounded))
            self.assertEqual(decision["sounding_roles"], sorted({e["layer"] for e in sounded}))
            self.assertEqual({e["harmonic_root"] for e in sounded}, {decision["selected_root"]})
        self.assertGreater(max(t["recurrent_rms"] for t in score["neural_trace"]), .01)
        self.assertGreater(len({d["selected_span_beats"] for d in score["decision_log"]}), 2)
        self.assertGreater(len(set(score["progression"])), 2)
        self.assertGreater(len({tuple(d["sounding_roles"]) for d in score["decision_log"]}), 1)
        self.assertGreater(len({d["memory_operation"] for d in score["decision_log"]}), 1)

    def test_memory_describes_notes_and_rhythm_that_actually_sounded(self):
        score = write_performance(self.circuit, 2026)
        tonic = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B").index(score["tonic"])
        scale = SCALES[score["mode"]]
        for event in score["events"]:
            self.assertEqual(midi(event["scale_degree"], tonic, scale, 60), event["midi"])
            if event["layer"] in ("bass", "harmony", "texture"):
                chord = {midi(event["harmonic_root"] + d, tonic, scale, 0) % 12 for d in (0, 2, 4)}
                self.assertIn(event["midi"] % 12, chord)
        for decision in score["decision_log"]:
            primary = [e for e in score["events"] if e["decision"] == decision["index"] and e["layer"] == decision["foreground"]]
            self.assertEqual(decision["performed_degrees"], [e["scale_degree"] for e in primary])
            steps = [(b["start_seconds"] - a["start_seconds"]) * score["bpm"] / 60 for a, b in zip(primary, primary[1:])]
            steps.append(primary[-1]["duration_seconds"] * score["bpm"] / 60 + .15)
            np.testing.assert_allclose(decision["performed_rhythm_steps"], steps, atol=1e-12)
        self.assertLessEqual(len(score["musical_memory"]), 6)
        for cell in score["musical_memory"]:
            decision = score["decision_log"][cell["phrase"]]
            self.assertTrue(decision["stored_new_cell"])
            self.assertEqual(cell["degrees"], decision["performed_degrees"][:6])
            self.assertEqual(cell["steps"], decision["performed_rhythm_steps"][:6])
        # Memory provenance references earlier material, including cells that have
        # since left the bounded working store.
        for decision in score["decision_log"]:
            if decision["memory_source_phrase"] is not None:
                self.assertLess(decision["memory_source_phrase"], decision["index"])
                self.assertTrue(score["decision_log"][decision["memory_source_phrase"]]["stored_new_cell"])

    def test_wiring_interventions_and_seed_change_actual_performances(self):
        voices = ENSEMBLES["luminous"]
        for seed in (42, 2026):
            captures = {c: write_performance(self.circuit, seed, 40, voices, c)
                        for c in ("connectome", "shuffled", "baseline", "disconnected")}
            ledgers = set()
            for capture in captures.values():
                self.assert_technical_capture(capture, 40)
                self.assertEqual(capture["voices"], voices)
                ledgers.add(tuple((e["layer"], e["midi"], e["start_seconds"], e["duration_seconds"]) for e in capture["events"]))
            self.assertEqual(len(ledgers), 4)
            self.assertEqual(max(t["recurrent_rms"] for t in captures["disconnected"]["neural_trace"]), 0)
            for key in ("input_projection_sha256", "readout_sha256"):
                self.assertEqual(len({s["controller"][key] for s in captures.values()}), 1)
        self.assertNotEqual(write_performance(self.circuit, 42)["event_sha256"],
                            write_performance(self.circuit, 2026)["event_sha256"])

    def test_neural_palette_uses_real_compatible_catalog_voices(self):
        catalog = {v["id"] for v in json.loads((ROOT / "data/instruments.json").read_text(encoding="utf8"))}
        palettes = set()
        for seed in (0, 1, 42, 77, 2026):
            score = write_performance(self.circuit, seed, 15)
            palettes.add(tuple(score["voices"][r] for r in ROLES))
            for role, voice in score["voices"].items():
                self.assertIn(voice, VOICE_POOLS[role])
                self.assertTrue(voice in catalog or voice == "classic:bass")
        self.assertGreater(len(palettes), 1)
        self.assertEqual(write_performance(self.circuit, voices=ENSEMBLES["luminous"])["voices"], ENSEMBLES["luminous"])

    def test_invalid_capture_requests_fail_before_rendering(self):
        for duration in (0, 9, 181, float("inf"), float("nan"), True, "75"):
            with self.subTest(duration=duration), self.assertRaises(ValueError):
                write_performance(self.circuit, seconds=duration)
        for seed in (-1, 2**31, True, 1.5):
            with self.subTest(seed=seed), self.assertRaises(ValueError):
                write_performance(self.circuit, seed=seed)
        with self.assertRaises(ValueError):
            write_performance(self.circuit, voices={"lead": "piano"})

    def test_continuation_keeps_source_notes_and_recovers_the_cropped_gesture_state(self):
        source = write_performance(self.circuit)
        untouched = copy.deepcopy(source)
        continued = extend_performance(self.circuit, source)
        self.assertEqual(source, untouched)
        self.assertEqual(source["event_sha256"], "a2b3f4c406699e42c7f1616516c11deccbaa33f9eda2d36f75a5d3fad2d02604")
        self.assertEqual(continued["events"][:len(source["events"])], source["events"])
        for key in ("phrases", "decision_log", "neural_trace", "progression"):
            self.assertEqual(continued[key][:len(source[key])], source[key], key)
        self.assert_technical_capture(continued, 120)
        context = continued["continuation"]
        checkpoint = context["checkpoints"][-1]
        self.assertEqual(context["source_event_sha256"], source["event_sha256"])
        self.assertEqual(context["new_notes"], len(continued["events"]) - len(source["events"]))
        self.assertTrue(context["target_reached"])
        self.assertEqual((context["target_seconds"], context["actual_seconds"]), (120, 120))
        self.assertTrue(checkpoint["retained_final_gesture"]["capture_limited"])
        self.assertEqual(checkpoint["retained_final_gesture"]["captured_span_beats"], source["decision_log"][-1]["captured_span_beats"])
        self.assertLess(checkpoint["resume_at_seconds"], source["duration_seconds"])
        first_new = continued["events"][len(source["events"])]
        self.assertAlmostEqual(first_new["start_seconds"], checkpoint["resume_at_seconds"])
        self.assertGreaterEqual(first_new["start_seconds"], checkpoint["source_last_note_end_seconds"])
        self.assertTrue(any(e["start_seconds"] > 75 for e in continued["events"][len(source["events"]):]))
        replay = Controller(self.circuit, source["seed"], source["condition"])
        for trace in source["neural_trace"]:
            outputs = replay.phrase(trace["phrase"], trace["inputs"][4], round(trace["inputs"][5] * 6))
            np.testing.assert_array_equal(outputs, trace["outputs"])
        self.assertEqual(checkpoint["controller_state_sha256"], fingerprint(replay.state))
        self.assertEqual(checkpoint["working_memory_sha256"], hashlib.sha256(json.dumps(source["musical_memory"], sort_keys=True).encode()).hexdigest())
        next_decision = continued["decision_log"][len(source["decision_log"])]
        outputs = replay.phrase(next_decision["index"], next_decision["input_feedback_energy"], next_decision["input_harmonic_root"])
        np.testing.assert_array_equal(outputs, next_decision["outputs"])
        self.assertTrue(any(d["memory_source_phrase"] is not None and d["memory_source_phrase"] < len(source["decision_log"])
                            for d in continued["decision_log"][len(source["decision_log"]):]))
        # A fresh120s run has a different uncropped source ending. It is not an
        # acceptable substitute for replaying the actual saved75s performance.
        fresh = write_performance(self.circuit, seconds=120)
        self.assertNotEqual(fresh["events"][:len(source["events"])], source["events"])

    def test_continuation_is_recursive_and_deterministic_across_seeds(self):
        for seed, condition in ((0, "connectome"), (42, "shuffled"), (2026, "disconnected")):
            source = write_performance(self.circuit, seed, 37.25, ENSEMBLES["luminous"], condition)
            longer = extend_performance(self.circuit, source, 90)
            longest = extend_performance(self.circuit, longer, 120)
            self.assertEqual(longer["events"][:len(source["events"])], source["events"])
            self.assertEqual(longest["events"][:len(longer["events"])], longer["events"])
            self.assertEqual(longest["continuation"]["capture_limits_seconds"], [37.25, 90., 120.])
            self.assertEqual(longest["event_sha256"], extend_performance(self.circuit, longer, 120)["event_sha256"])
            self.assert_technical_capture(longest, 120)

    def test_fractional_capture_exit_retains_the_unwritten_controller_update(self):
        # This limit leaves0.885beats after a real state update, below the gesture
        # interpreter's0.9beat minimum. Old capture has9neural steps but8gestures.
        source = write_performance(self.circuit, seconds=23.021100917431195)
        self.assertEqual((len(source["decision_log"]), len(source["neural_trace"])), (8, 9))
        continued = extend_performance(self.circuit, source)
        self.assertEqual(continued["events"][:len(source["events"])], source["events"])
        self.assertEqual(continued["neural_trace"][:9], source["neural_trace"])
        checkpoint = continued["continuation"]["checkpoints"][-1]
        self.assertEqual(checkpoint["unwritten_controller_steps"], 1)
        self.assertEqual(checkpoint["next_controller_trace_index"], 9)
        self.assertEqual(continued["decision_log"][8]["controller_trace_index"], 9)
        replay = Controller(self.circuit, source["seed"])
        for trace in source["neural_trace"]:
            replay.phrase(trace["phrase"], trace["inputs"][4], round(trace["inputs"][5] * 6))
        self.assertEqual(fingerprint(replay.state), checkpoint["controller_state_sha256"])
        next_decision = continued["decision_log"][8]
        np.testing.assert_array_equal(replay.phrase(next_decision["index"], next_decision["input_feedback_energy"],
                                                  next_decision["input_harmonic_root"]), next_decision["outputs"])
        self.assert_technical_capture(continued, 120)

    def test_continuation_refuses_changed_source_notes_trace_memory_and_history(self):
        source = write_performance(self.circuit)
        variants = []
        edited = copy.deepcopy(source); edited["events"][0]["midi"] += 1
        edited["event_sha256"] = hashlib.sha256(json.dumps(edited["events"], sort_keys=True).encode()).hexdigest()
        variants.append(edited)
        edited = copy.deepcopy(source); edited["neural_trace"][0]["outputs"][0] += .01; variants.append(edited)
        edited = copy.deepcopy(source); edited["musical_memory"][0]["degrees"][0] += 1; variants.append(edited)
        edited = copy.deepcopy(source); edited["voices"]["lead"] = "piano"; variants.append(edited)
        edited = copy.deepcopy(source); edited["decoder_provenance"] = None; variants.append(edited)
        edited = extend_performance(self.circuit, source, 90)
        edited["continuation"]["capture_limits_seconds"] = [75, 85]; variants.append(edited)
        for edited in variants:
            with self.assertRaises(ValueError):
                extend_performance(self.circuit, edited)
        for duration in (75, 79, 181, float("inf"), True):
            with self.assertRaises(ValueError):
                extend_performance(self.circuit, source, duration)
        different_graph = {**self.circuit, "weights": self.circuit["weights"].copy()}
        different_graph["weights"][0] *= 3
        with self.assertRaises(ValueError):
            extend_performance(different_graph, source)
        labeled = {**source, "title": "Listener's title", "library_note": "External annotation"}
        self.assertEqual(extend_performance(self.circuit, labeled)["event_sha256"], extend_performance(self.circuit, source)["event_sha256"])

    def test_continuation_places_old_and_new_notes_in_valid_recording_frames(self):
        source = write_performance(self.circuit)
        continued = extend_performance(self.circuit, source)
        rate = 400
        def carrier(event, sample_rate):
            return 1. + (np.arange(int(event["duration_seconds"] * sample_rate)) + event["midi"]) % 5
        old_audio, old_stems, old_audit = render_score(source, carrier, rate)
        new_audio, new_stems, audit = render_score(continued, carrier, rate)
        self.assertEqual(audit["notes_mixed"], len(continued["events"]))
        self.assertEqual(audit["events"][:len(source["events"])], old_audit["events"])
        resume_frame = int(continued["continuation"]["resume_at_seconds"] * rate)
        np.testing.assert_array_equal(new_audio[:resume_frame], old_audio[:resume_frame])
        for role in ROLES:
            np.testing.assert_array_equal(new_stems[role][:resume_frame], old_stems[role][:resume_frame])
        self.assertTrue(np.any(new_audio[75 * rate:119 * rate] != 0))
        self.assertTrue(np.all(new_audio[-rate:] == 0))
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "continuation.wav"
            save_wav(target, new_audio * .5, rate)
            with wave.open(str(target)) as recording:
                self.assertEqual(recording.getnframes(), 120 * rate)
                self.assertEqual(recording.getframerate(), rate)


if __name__ == "__main__":
    unittest.main()

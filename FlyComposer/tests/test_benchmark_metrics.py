import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from flycomposer.benchmark_metrics import SR, measure


def fixture(specs, bpm=60, recorded_seconds=41):
    events = []
    for index, spec in enumerate(specs):
        part, pitch, start, duration, *optional = spec
        voice = optional[0] if optional else "fm:lead"
        events.append({"id": index, "layer": part, "voice": voice, "midi": pitch,
                       "start_seconds": start, "duration_seconds": duration,
                       "start_beat": start * bpm / 60, "length_beats": duration * bpm / 60, "velocity": .7})
    digest = hashlib.sha256(json.dumps(events, sort_keys=True).encode()).hexdigest()
    score = {"bpm": bpm, "events": events, "voices": {e["layer"]: e["voice"] for e in events}, "event_sha256": digest}
    ledger = [{**event, "start_frame": int(event["start_seconds"] * SR), "frames": int(event["duration_seconds"] * SR),
               "end_frame": int(event["start_seconds"] * SR) + int(event["duration_seconds"] * SR),
               "mixed": True, "placed_rms": .05, "note_pcm_sha256": "0" * 64} for event in events]
    audit = {"events": ledger, "sample_rate": SR, "frames": int(recorded_seconds * SR),
             "notes_scheduled": len(events), "notes_mixed": len(events), "event_sha256": digest, "complete": True}
    return score, audit


class BenchmarkFactsTests(unittest.TestCase):
    def test_touching_frame_intervals_do_not_invent_polyphony(self):
        score, audit = fixture([("lead", 60, .1, .2), ("answer", 64, .3, .2)])
        self.assertGreater(.1 + .2, .3)  # Floats appear to overlap; renderer frames do not.
        result = measure(score, audit, 1)
        self.assertEqual(result["pitched_parts"]["peak"], 1)
        self.assertEqual(result["pitched_parts"]["fraction_at_least_2"], 0)
        self.assertAlmostEqual(result["occupied_fraction"], .4)
        self.assertTrue(result["delivery"]["complete"])

    def test_chord_and_parallel_parts_have_different_layer_counts(self):
        chord = fixture([("harmony", pitch, 0, 10) for pitch in (60, 64, 67)])
        parts = fixture([(part, pitch, 0, 10) for part, pitch in zip(("lead", "answer", "bass"), (60, 64, 48))])
        one, three = measure(*chord, 10), measure(*parts, 10)
        self.assertEqual(one["pitched_parts"]["mean"], 1)
        self.assertEqual(one["pitched_note_polyphony"]["mean"], 3)
        self.assertEqual(three["pitched_parts"]["fraction_at_least_3"], 1)
        self.assertEqual(three["pitched_note_polyphony"], one["pitched_note_polyphony"])

    def test_target_window_includes_tail_and_clips_overflow(self):
        score, audit = fixture([("lead", 60, 2, 3), ("bass", 48, 8, 4)], recorded_seconds=14)
        early = measure(score, audit, 8)
        self.assertEqual(early["trailing_gap_seconds"], 3)
        result = measure(score, audit, 10)
        self.assertEqual(result["musical_end_seconds"], 12)
        self.assertEqual(result["leading_gap_seconds"], 2)
        self.assertEqual(result["longest_gap_seconds"], 3)
        self.assertEqual(result["trailing_gap_seconds"], 0)
        self.assertEqual(result["occupied_fraction"], .5)
        self.assertEqual(result["notes_extending_past_target"], 1)
        self.assertEqual(early["notes_starting_after_target"], 1)

    def test_repeated_scale_is_separate_from_answer_and_empty_windows(self):
        specs = [("lead", pitch, phrase * 8 + beat, 1) for phrase in range(2)
                 for beat, pitch in enumerate((60, 62, 64, 65, 67, 69, 71, 72))]
        specs += [("answer", 80 - beat * 2, beat + .5, .25) for beat in range(8)]
        result = measure(*fixture(specs), 24)
        lead = result["foreground"]["lead"]
        self.assertEqual(lead["longest_monotonic_step_run_notes"], 8)
        self.assertEqual(lead["exact_repeated_phrase_share"], .5)
        self.assertEqual(lead["adjacent_exact_repeat_share"], 1)
        self.assertEqual(lead["nonempty_two_bar_windows"], 2)
        self.assertIsNone(lead["exact_fingerprints"][2])
        self.assertEqual(result["foreground"]["answer"]["exact_repeated_phrase_share"], 0)

    def test_drums_do_not_count_as_pitched_parts_but_pitched_texture_does(self):
        score, audit = fixture([("lead", 60, 0, 10), ("texture", 36, 0, 10, "chip:noise")])
        self.assertEqual(measure(score, audit, 10)["pitched_parts"]["mean"], 1)
        score, audit = fixture([("lead", 60, 0, 10), ("texture", 72, 0, 10, "celesta")])
        self.assertEqual(measure(score, audit, 10)["pitched_parts"]["mean"], 2)

    def test_missing_mixed_note_and_modified_pitch_fail_delivery(self):
        score, audit = fixture([("lead", 60, 0, 4), ("answer", 64, 0, 4)])
        original = copy.deepcopy((score, audit))
        result = measure(score, audit, 8)
        self.assertTrue(result["delivery"]["complete"])
        self.assertEqual((score, audit), original)
        audit["events"][1]["mixed"] = False
        result = measure(score, audit, 8)
        self.assertFalse(result["delivery"]["complete"])
        self.assertEqual(result["delivery"]["notes_mixed"], 1)
        self.assertEqual(result["pitched_parts"]["mean"], .5)
        audit["events"][1]["mixed"] = True
        audit["events"][1]["midi"] = 65
        self.assertFalse(measure(score, audit, 8)["delivery"]["exact_authored_notes"])

    def test_quarters_and_empty_score_are_json_serializable(self):
        score, audit = fixture([("lead", 60, 0, 1), ("answer", 64, 3, 1), ("bass", 48, 7, 1)])
        result = measure(score, audit, 8)
        self.assertEqual([row["notes"] for row in result["quarters"]], [1, 1, 0, 1])
        self.assertIsNone(result["quarters"][2]["mean_velocity"])
        empty = measure(*fixture([]), 8)
        self.assertEqual(empty["longest_gap_seconds"], 8)
        self.assertFalse(empty["delivery"]["complete"])
        json.dumps([result, empty], allow_nan=False)

    def test_wrong_frame_count_and_silent_pcm_fail_delivery(self):
        score, audit = fixture([("lead", 60, 0, 1)])
        audit["events"][0]["frames"] -= 1
        audit["events"][0]["end_frame"] -= 1
        self.assertFalse(measure(score, audit, 8)["delivery"]["exact_frame_placements"])
        score, audit = fixture([("lead", 60, 0, 1)])
        audit["events"][0]["placed_rms"] = 0
        self.assertFalse(measure(score, audit, 8)["delivery"]["nonzero_note_pcm_evidence"])


if __name__ == "__main__":
    unittest.main()

"""Musical development regressions using the packaged, attributed real graph.

These tests check complete timing, distinguishable musical structures and circuit
influence. They intentionally make no claim about listener preference or beauty.
"""
import hashlib
import json
from pathlib import Path
import sys
import unittest

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from flycomposer.composition import ENSEMBLES, ROLES, SCALES, assess_score, midi
from flycomposer.connectome import load_circuit
from flycomposer.development import DIRECTIONS, write_development
from flycomposer.render import LEVEL, render_score


class DevelopmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.circuit = load_circuit()

    def test_each_direction_is_complete_across_seeds_and_has_a_tonic_ending(self):
        for direction in DIRECTIONS:
            for seed in (0, 1, 7, 42, 77, 2026, 2**31 - 1):
                with self.subTest(direction=direction, seed=seed):
                    score = write_development(self.circuit, seed, direction=direction)
                    facts = assess_score(score)
                    self.assertGreaterEqual(facts["unique_lead_phrases"], 6)
                    self.assertEqual(facts["scalar_runs"], 0)
                    self.assertGreater(facts["lead_rest_fraction"], .20)
                    self.assertEqual(score["voices"], ENSEMBLES["luminous"])
                    self.assertEqual(score["progression"][-2:], [4, 0])
                    lead = [e for e in score["events"] if e["layer"] == "lead"]
                    bass = [e for e in score["events"] if e["layer"] == "bass"]
                    self.assertEqual(lead[-1]["midi"] % 12, bass[0]["midi"] % 12)
                    self.assertGreater(lead[-1]["duration_seconds"], 1)
                    last_end = max(e["start_seconds"] + e["duration_seconds"] for e in score["events"])
                    self.assertGreater(score["duration_seconds"] - last_end, 1.05)
                    self.assertEqual([e["id"] for e in score["events"]], list(range(len(score["events"]))))
                    self.assertEqual(score["sections"][0]["start_seconds"], 0)
                    self.assertAlmostEqual(score["sections"][-1]["end_seconds"], score["duration_seconds"] - 1)
                    for a, b in zip(score["sections"], score["sections"][1:]):
                        self.assertEqual(a["end_seconds"], b["start_seconds"])
                    # Fingerprints must reflect the notes actually heard, not
                    # phrase labels or offsets invented to satisfy a metric.
                    for phrase in score["phrases"]:
                        start = phrase["start_beat"]
                        sounding = [e for e in lead if start <= e["start_beat"] < start + phrase["length_beats"]]
                        data = [(e["midi"], round(e["start_beat"] - start, 5), e["length_beats"]) for e in sounding]
                        self.assertEqual(phrase["lead_notes"], len(sounding))
                        self.assertEqual(phrase["lead_signature"], hashlib.sha256(json.dumps(data).encode()).hexdigest())

    def test_directions_change_meter_pace_density_and_form_with_same_palette(self):
        scores = {d: write_development(self.circuit, 42, direction=d) for d in DIRECTIONS}
        slow, tide, groove = (scores[d] for d in ("nocturne", "tide", "sparks"))
        self.assertEqual([s["meter"] for s in scores.values()], ["4/4", "6/8", "4/4"])
        self.assertEqual(tide["beats_per_bar"], 3)
        self.assertAlmostEqual(tide["pulse_bpm"], tide["bpm"] * 2 / 3)
        self.assertLess(slow["bpm"], groove["bpm"] - 30)
        density = {d: len(s["events"]) / s["duration_seconds"] for d, s in scores.items()}
        self.assertGreater(density["tide"], density["nocturne"] * 1.6)
        self.assertGreater(density["sparks"], density["nocturne"] * 2)
        self.assertEqual(len(groove["sections"]), 5)
        self.assertTrue(any(p["length_beats"] == 4 for p in groove["phrases"]))
        self.assertTrue(any(p["length_beats"] == 12 for p in slow["phrases"]))
        self.assertTrue(any(p["length_beats"] == 9 for p in tide["phrases"]))
        self.assertEqual(len({tuple(s["progression"]) for s in scores.values()}), 3)
        for score in scores.values():
            self.assertEqual(score["voices"], slow["voices"])
            first_phrase_end = score["phrases"][0]["length_beats"]
            entrance_roles = {e["layer"] for e in score["events"] if e["start_beat"] < first_phrase_end}
            self.assertLess(len(entrance_roles), 5)
            self.assertEqual({e["layer"] for e in score["events"]}, set(ROLES))
        # The groove's quiet middle is a real density withdrawal in all notes.
        section_density = []
        for section in groove["sections"]:
            n = sum(section["start_seconds"] <= e["start_seconds"] < section["end_seconds"] for e in groove["events"])
            section_density.append(n / (section["end_seconds"] - section["start_seconds"]))
        self.assertLess(section_density[2], section_density[1] * .5)
        self.assertLess(section_density[2], section_density[3] * .5)

    def test_seed_changes_theme_harmony_rhythm_and_sectional_form(self):
        for direction in DIRECTIONS:
            scores = [write_development(self.circuit, seed, direction=direction) for seed in (1, 7, 42, 77, 2026)]
            self.assertGreaterEqual(len({tuple(s["motif"]) for s in scores}), 4)
            self.assertGreaterEqual(len({tuple(s["progression"]) for s in scores}), 4)
            forms = {tuple(section["end_beat"] - section["start_beat"] for section in s["sections"]) for s in scores}
            self.assertGreaterEqual(len(forms), 2)
            rhythm = {tuple((e["start_beat"], e["length_beats"]) for e in s["events"] if e["layer"] == "lead") for s in scores}
            self.assertEqual(len(rhythm), len(scores))
            self.assertEqual(scores[2]["event_sha256"], write_development(self.circuit, 42, direction=direction)["event_sha256"])

    def test_supporting_parts_follow_their_active_chord_when_harmony_moves(self):
        for direction in DIRECTIONS:
            for seed in (1, 7, 42, 77, 2026):
                score = write_development(self.circuit, seed, direction=direction)
                tonic = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B").index(score["tonic"])
                scale = SCALES[score["mode"]]
                for event in score["events"]:
                    if event["layer"] in ("bass", "harmony", "texture"):
                        allowed = {midi(event["harmonic_root"] + degree, tonic, scale, 0) % 12 for degree in (0, 2, 4)}
                        self.assertIn(event["midi"] % 12, allowed, (direction, seed, event))
                # In Nocturne's crest the later bass/texture must follow the
                # chord actually held by the harmony at that position.
                if direction == "nocturne":
                    for phrase in score["phrases"]:
                        if phrase["section"] != 2:
                            continue
                        phrase_start = phrase["start_beat"]
                        change = phrase_start + phrase["length_beats"] * .53
                        phrase_end = phrase_start + phrase["length_beats"]
                        later = [e for e in score["events"] if change <= e["start_beat"] < phrase_end and e["layer"] in ("bass", "harmony", "texture")]
                        self.assertTrue(later)
                        self.assertEqual(len({e["harmonic_root"] for e in later}), 1)

    def test_circuit_changes_actual_notes_with_an_identical_exogenous_brief(self):
        for direction in DIRECTIONS:
            scores = {c: write_development(self.circuit, 42, direction=direction, condition=c)
                      for c in ("connectome", "shuffled", "baseline", "disconnected")}
            original = scores["connectome"]
            for score in scores.values():
                assess_score(score)
                for key in ("motif", "progression", "sections", "bpm", "voices"):
                    self.assertEqual(score[key], original[key])
                for key in ("input_projection_sha256", "readout_sha256"):
                    self.assertEqual(score["controller"][key], original["controller"][key])
            self.assertEqual(len({s["event_sha256"] for s in scores.values()}), 4)
            lead_ledgers = {tuple((e["midi"], e["start_beat"], e["length_beats"]) for e in s["events"] if e["layer"] == "lead")
                            for s in scores.values()}
            self.assertEqual(len(lead_ledgers), 4)
            self.assertGreater(max(t["recurrent_rms"] for t in original["neural_trace"]), 0)
            self.assertEqual(max(t["recurrent_rms"] for t in scores["disconnected"]["neural_trace"]), 0)

    def test_new_meter_and_phrase_lengths_render_every_note_in_the_correct_stem(self):
        # Simple nonzero carrier lets the test independently reconstruct every
        # stem's occupied samples, including fractional quarter-note boundaries.
        sample_rate = 800
        for direction in DIRECTIONS:
            score = write_development(self.circuit, 77, direction=direction)
            audio, stems, audit = render_score(score, lambda e, sr: np.ones(int(e["duration_seconds"] * sr)), sample_rate)
            expected = {role: np.zeros(len(audio)) for role in ROLES}
            for event in score["events"]:
                begin = int(event["start_seconds"] * sample_rate)
                count = int(event["duration_seconds"] * sample_rate)
                expected[event["layer"]][begin:begin + count] += .08 * LEVEL[event["layer"]] * event["velocity"]
            for role in ROLES:
                np.testing.assert_allclose(stems[role], expected[role], atol=1e-14)
                self.assertGreater(np.count_nonzero(stems[role]), 0)
            self.assertEqual(audit["notes_mixed"], len(score["events"]))
            self.assertTrue(all(e["mixed"] for e in audit["events"]))
            self.assertTrue(np.all(audio[-sample_rate:] == 0))

    def test_invalid_direction_and_incomplete_voice_maps_fail_before_composition(self):
        with self.assertRaises(ValueError):
            write_development(self.circuit, direction="unknown")
        with self.assertRaises(ValueError):
            write_development(self.circuit, voices={"lead": "electric_piano"})
        with self.assertRaises(ValueError):
            write_development(self.circuit, seed=True)


class ConnectedSparksTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.circuit = load_circuit()

    def pair(self, seed=2026, condition="connectome"):
        original = write_development(self.circuit, seed, direction="sparks", condition=condition)
        revised = write_development(self.circuit, seed, direction="sparks", condition=condition,
                                    revision="connected_phrases_v1")
        return original, revised

    def test_saved_v1_ledgers_reproduce_exactly_and_revision_is_explicit(self):
        # Captured before introducing this revision: saved listening comparisons
        # depend on the exact v1 events, not merely similar summary statistics.
        references = {
            ("nocturne", 42): "809f031f10d82cdeb78561ec13c771606ff7201099db47aacf755621debce263",
            ("nocturne", 77): "1513b2b41ef3ccbca095371986162a1efc364f4999c55690cade9881a0171c2c",
            ("nocturne", 2026): "1dc845204e5d3b4c1caf50963b42e10494a719351c0b06152c232688799501a0",
            ("tide", 42): "742582723ea0ee546924d0b07514ea4144a497f3717f015430331385539a831a",
            ("tide", 77): "951731487ebd019a0af4b9c1d54cafe13230576dca80801c4c2338ef591362a2",
            ("tide", 2026): "cb3798fd8799e4762e597e98108270ab8c924bdfbc92278f9ace3aed5e7fc6b8",
            ("sparks", 42): "5c122405e51528ce97ae88146851e7f92714fd77ed3e6176f6549c411324a487",
            ("sparks", 77): "8abf0b64467ff514e86bd1f33c5f31ce84afff708754112e22b565d0ecfadba9",
            ("sparks", 2026): "2c68d0b681309f7eb40a2e4c38f561a62964e3de5cd937f794a234f9b9b37a16",
        }
        for (direction, seed), expected in references.items():
            default = write_development(self.circuit, seed, direction=direction)
            explicit = write_development(self.circuit, seed, direction=direction, revision=None)
            self.assertEqual(default, explicit)
            self.assertEqual(default["event_sha256"], expected)
            self.assertNotIn("revision", default)
        for direction, revision in (("tide", "connected_phrases_v1"), ("sparks", "invented")):
            with self.assertRaises(ValueError):
                write_development(self.circuit, direction=direction, revision=revision)

    def test_revision_keeps_the_piece_and_accompaniment_while_changing_foreground(self):
        original, revised = self.pair()
        for key in ("seed", "condition", "voices", "motif", "companion_motif", "progression", "sections",
                    "meter", "beats_per_bar", "bpm", "bars", "duration_seconds", "controller", "neural_trace"):
            self.assertEqual(revised[key], original[key], key)
        self.assertEqual(revised["revision"], "connected_phrases_v1")
        self.assertEqual(revised["revision_source_event_sha256"], original["event_sha256"])
        self.assertNotEqual(revised["event_sha256"], original["event_sha256"])
        self.assertEqual(len(revised["events"]), len(original["events"]))
        self.assertEqual(revised["composition_rules"]["continuity_revision"]["added_notes"], 0)
        for role in ("bass", "harmony", "texture"):
            old = [{k: v for k, v in e.items() if k != "id"} for e in original["events"] if e["layer"] == role]
            new = [{k: v for k, v in e.items() if k != "id"} for e in revised["events"] if e["layer"] == role]
            self.assertEqual(new, old, role)
        for part in ("lead", "answer"):
            self.assertNotEqual([e for e in revised["events"] if e["layer"] == part],
                                [e for e in original["events"] if e["layer"] == part])
        # The opening motif's pitches remain recognizable; it has room to speak.
        old_opening = [e for e in original["events"] if e["layer"] == "lead" and e["start_beat"] < 4]
        new_opening = [e for e in revised["events"] if e["layer"] == "lead" and e["start_beat"] < 4]
        self.assertEqual([e["midi"] for e in old_opening], [e["midi"] for e in new_opening])
        self.assertTrue(all(e["length_beats"] >= .5 for e in new_opening))

    def test_opening_and_voice_handoffs_connect_without_erasing_the_breakdown(self):
        original, revised = self.pair()

        def opening_gap(score):
            notes = [e for e in score["events"] if e["layer"] in ("lead", "answer") and e["start_beat"] < 12]
            gaps, ending = [], 0.
            for event in notes:
                gaps.append(max(0., event["start_beat"] - ending))
                ending = max(ending, event["start_beat"] + event["length_beats"])
            return max(gaps)

        self.assertGreater(opening_gap(original), 4)
        self.assertLess(opening_gap(revised), 1.5)
        old_leaps, new_leaps = [], []
        for score, leaps in ((original, old_leaps), (revised, new_leaps)):
            for phrase in score["phrases"]:
                begin, end = phrase["start_beat"], phrase["start_beat"] + phrase["length_beats"]
                lead = [e for e in score["events"] if e["layer"] == "lead" and begin <= e["start_beat"] < end]
                answer = [e for e in score["events"] if e["layer"] == "answer" and lead[-1]["start_beat"] < e["start_beat"] < end]
                if answer:
                    leaps.append(abs(answer[0]["midi"] - lead[-1]["midi"]))
        self.assertLess(np.mean(new_leaps), np.mean(old_leaps) * .5)
        self.assertLessEqual(max(new_leaps), 7)
        for phrase in revised["phrases"]:
            if phrase["section"] != 2:
                continue
            begin, end = phrase["start_beat"], phrase["start_beat"] + phrase["length_beats"]
            notes = [e for e in revised["events"] if begin <= e["start_beat"] < end]
            lead = [e for e in notes if e["layer"] == "lead"]
            answer = [e for e in notes if e["layer"] == "answer"]
            handoff_gap = lead[0]["start_beat"] - answer[0]["start_beat"] - answer[0]["length_beats"]
            self.assertAlmostEqual(handoff_gap, .25)
            self.assertAlmostEqual(end - lead[-1]["start_beat"] - lead[-1]["length_beats"], .25)
        # The quiet section retains its exact note count, the groove its offbeats,
        # and foreground silence still occupies a substantial part of the piece.
        for section in range(5):
            self.assertEqual(sum(e["section"] == section for e in revised["events"]),
                             sum(e["section"] == section for e in original["events"]))
        facts = assess_score(revised)
        self.assertGreater(facts["lead_rest_fraction"], .35)
        onset_fraction = [e["start_beat"] % 1 for e in revised["events"] if e["layer"] == "lead"]
        self.assertGreater(sum(value != 0 for value in onset_fraction), len(onset_fraction) * .6)
        tonic = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B").index(revised["tonic"])
        pitch_degrees = {midi(d, tonic, SCALES[revised["mode"]], 60): d for d in range(-21, 28)}
        for echo in revised["composition_rules"]["continuity_revision"]["paired_theme_echoes"]:
            intervals = []
            for index in (echo["source_phrase"], echo["phrase"]):
                phrase = revised["phrases"][index]
                start = phrase["start_beat"]
                head = [e for e in revised["events"] if e["layer"] == "lead" and start <= e["start_beat"] < start + phrase["length_beats"]][:2]
                intervals.append(pitch_degrees[head[1]["midi"]] - pitch_degrees[head[0]["midi"]])
            self.assertEqual(intervals, [echo["diatonic_interval"]] * 2)

    def test_revision_keeps_complete_timing_harmony_and_endings_across_controls(self):
        for seed in (0, 1, 42, 77, 2026, 2**31 - 1):
            for condition in ("connectome", "shuffled", "baseline", "disconnected"):
                with self.subTest(seed=seed, condition=condition):
                    original, revised = self.pair(seed, condition)
                    facts = assess_score(revised)
                    self.assertEqual(facts["scalar_runs"], 0)
                    self.assertGreaterEqual(facts["unique_lead_phrases"], 6)
                    last = [e for e in revised["events"] if e["layer"] == "lead"][-1]
                    old_last = [e for e in original["events"] if e["layer"] == "lead"][-1]
                    self.assertEqual({k: v for k, v in last.items() if k != "id"},
                                     {k: v for k, v in old_last.items() if k != "id"})
                    for phrase in revised["phrases"]:
                        start, length = phrase["start_beat"], phrase["length_beats"]
                        lead = [e for e in revised["events"] if e["layer"] == "lead" and start <= e["start_beat"] < start + length]
                        data = [(e["midi"], round(e["start_beat"] - start, 5), e["length_beats"]) for e in lead]
                        self.assertEqual(phrase["lead_signature"], hashlib.sha256(json.dumps(data).encode()).hexdigest())
                    for event in revised["events"]:
                        self.assertGreater(event["duration_seconds"], 0)
                        self.assertGreaterEqual(event["midi"], 33)
                        self.assertLessEqual(event["midi"], 91)

    def test_revised_audio_uses_every_note_and_retains_backing_stems(self):
        original, revised = self.pair()
        rate = 800
        def carrier(event, sample_rate):
            return 1. + (np.arange(int(event["duration_seconds"] * sample_rate)) + event["midi"]) % 7
        old_audio, old_stems, old_audit = render_score(original, carrier, rate)
        new_audio, new_stems, audit = render_score(revised, carrier, rate)
        self.assertEqual(audit["notes_mixed"], 255)
        self.assertEqual(audit["notes_mixed"], old_audit["notes_mixed"])
        for role in ("bass", "harmony", "texture"):
            np.testing.assert_array_equal(new_stems[role], old_stems[role])
        self.assertFalse(np.array_equal(new_audio, old_audio))
        for role in ("lead", "answer"):
            expected = np.zeros(len(new_audio))
            for event in revised["events"]:
                if event["layer"] != role:
                    continue
                raw = carrier(event, rate)
                values = raw * (.08 * LEVEL[role] * event["velocity"] / np.sqrt(np.mean(raw * raw)))
                begin = int(event["start_seconds"] * rate)
                expected[begin:begin + len(values)] += values
            np.testing.assert_allclose(new_stems[role], expected, atol=1e-14)
        self.assertTrue(np.all(new_audio[-rate:] == 0))


if __name__ == "__main__":
    unittest.main()

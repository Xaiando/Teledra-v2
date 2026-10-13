import copy
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from flycomposer.organist import OrganistError, expand_authoring
from flycomposer.organist_study import _require_continuity, generate_study, score_facts
from flycomposer.render import assess_performance


class FreshStudyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "data").mkdir()
        (self.root / "profiles").mkdir()
        shutil.copy2(ROOT / "data/instruments.json", self.root / "data/instruments.json")
        shutil.copy2(ROOT / "profiles/organist-study.md", self.root / "profiles/organist-study.md")
        source = {"title": "Criticized recording", "bpm": 60,
                  "voices": {"lead": "classic:keys", "texture": "classic:pad"},
                  "clips": [{"part": "lead", "events": [[60, 0, .25, 1]], "placements": [[0, 0]]},
                            {"part": "texture", "events": [[67, .25, .25, 1]], "placements": [[0, 0]]}]}
        self.source = expand_authoring(source, {"classic:keys", "classic:pad"}, seed=123, seconds=75)
        self.source_path = self.root / "original-score.json"
        self.source_path.write_text(json.dumps(self.source), encoding="utf-8")
        self.source_bytes = self.source_path.read_bytes()
        self.calls = []
        self.feedback = "Organist is being lazy."

    def tearDown(self):
        self.tmp.cleanup()

    @staticmethod
    def note(part, pitch, beat, length, velocity):
        return {"part": part, "midi": pitch, "beat": beat, "length": length, "velocity": velocity}

    def fake_model(self, endpoint, payload, output_dir, attempt, progress):
        name = output_dir.name
        self.calls.append((name, copy.deepcopy(payload)))
        if name == "plan":
            response = {"title": "New interpretation", "idea": "Let the foreground breathe over held support",
                        "part_intentions": {"lead": "A deliberate foreground", "texture": "Continuous support"},
                        "passages": [{"name": "First phrase", "start_beat": 0, "end_beat": 14,
                                      "intent": "Establish the material"},
                                     {"name": "Answer", "start_beat": 12, "end_beat": 28,
                                      "intent": "Develop and settle"}]}
        elif name == "passage-0":
            response = {"title": "First phrase", "notes": [self.note("lead", 64, 0, 1, .6), self.note("texture", 48, 0, 14, .25)]}
        elif name == "passage-1":
            response = {"title": "Answer", "notes": [self.note("lead", 65, 14, 1, .6), self.note("texture", 50, 12, 16, .3)]}
        elif name == "model-review":
            response = {"critique": "The score gives the supporting part room; I will give the first arrival more time. This is score inspection, not listening.",
                        "replacements": [{"passage_index": 0, "reason": "Develop the first arrival",
                                          "notes": [self.note("lead", 67, 2, 4, .65), self.note("texture", 52, 0, 14, .25)]}]}
        else:
            raise AssertionError(name)
        (output_dir / f"request-{attempt}.json").write_text(json.dumps(payload), encoding="utf-8")
        (output_dir / f"authoring-{attempt}.txt").write_text(json.dumps(response), encoding="utf-8")
        return json.dumps(response), {"model": "actual-test-model", "eval_count": 100, "done_reason": "stop"}

    def generate(self, model=None):
        with patch("flycomposer.organist_study._stream_request", side_effect=model or self.fake_model), \
             patch.dict("os.environ", {"TELEDRA_OLLAMA_URL": "http://localhost:11434", "TELEDRA_MODEL": "configured-test-model"}):
            return generate_study(self.root, self.source, self.feedback, seconds=75)

    def assert_source_preserved(self, original):
        self.assertEqual(self.source, original)
        self.assertEqual(self.source_path.read_bytes(), self.source_bytes)

    def test_plan_author_review_revises_model_notes_and_preserves_source(self):
        original = copy.deepcopy(self.source)
        score = self.generate()
        self.assertEqual([phase for phase, _ in self.calls], ["plan", "passage-0", "passage-1", "model-review"])
        self.assert_source_preserved(original)
        self.assertEqual(score["seed"], original["seed"])
        self.assertEqual(score["voices"], original["voices"])
        self.assertEqual(score["bpm"], original["bpm"])
        self.assertEqual(score["duration_seconds"], 29)
        self.assertEqual(score["arrangement_version"], "organist_planned_study_v2")
        self.assertEqual(score["study"]["continuity_contract"]["max_silence_seconds"], 2)
        self.assertEqual(score["study"]["continuity_evidence"]["global_max_gap_seconds"], 0)
        self.assertEqual(score["study"]["revised_passages"], [0])
        self.assertEqual(score["study"]["feedback"], self.feedback)
        self.assertEqual(score["study"]["listener_verdict"], "unreviewed")
        self.assertEqual(score["inference_provenance"]["returned_models"], ["actual-test-model"])
        revised = [event for event in score["events"] if event["source_passage"] == 0]
        self.assertEqual((revised[0]["midi"], revised[0]["start_seconds"], revised[0]["duration_seconds"], revised[0]["velocity"]), (67, 2, 4, .65))
        unchanged = [event for event in score["events"] if event["source_passage"] == 1]
        self.assertEqual((unchanged[0]["midi"], unchanged[0]["start_seconds"], unchanged[0]["velocity"]), (65, 14, .6))
        assess_performance(score)
        directory = Path(score["study"]["output_dir"])
        self.assertEqual(json.loads((directory / "operator-feedback.json").read_text()), self.feedback)
        self.assertEqual(json.loads((directory / "source-score.json").read_text()), original)
        self.assertTrue((directory / "plan.json").exists())
        self.assertTrue((directory / "factual-critique.json").exists())
        self.assertIn("not listening", (directory / "model-critique.json").read_text())
        self.assertIn("criticized_source_facts", self.calls[0][1]["messages"][1]["content"])
        self.assertNotIn("source_motif_examples", json.dumps(self.calls))
        self.assertIn("Array order consumes no time", self.calls[1][1]["messages"][0]["content"])

    def test_layer_facts_distinguish_parallel_parts_from_serial_labels(self):
        catalog = json.loads((self.root / "data/instruments.json").read_text())
        serial = score_facts(self.source["events"], 60, catalog)
        self.assertEqual(serial["global_max_polyphony"], 1)
        self.assertEqual(serial["cross_part_overlap_beats"], 0)
        layered = copy.deepcopy(self.source["events"])
        layered[1]["start_seconds"] = 0
        layered[1]["start_beat"] = 0
        facts = score_facts(layered, 60, catalog)
        self.assertEqual(facts["global_max_polyphony"], 2)
        self.assertEqual(facts["cross_part_overlap_beats"], .25)
        self.assertEqual(facts["parts"]["lead"]["occupied_beats"], .25)

    def test_invalid_passage_timing_gets_two_repairs_without_note_clamping(self):
        original = copy.deepcopy(self.source)
        def outside(*args):
            raw, inference = self.fake_model(*args)
            if args[2].name == "passage-1":
                response = json.loads(raw)
                response["notes"][0]["beat"] = 32
                response["notes"][1]["beat"] = 30
                return json.dumps(response), inference
            return raw, inference
        with self.assertRaises(OrganistError) as caught:
            self.generate(outside)
        self.assertEqual(sum(name == "passage-1" for name, _ in self.calls), 3)
        self.assertIn("n0:start=32,end=33", str(caught.exception))
        self.assertIn("n1:start=30,end=46", str(caught.exception))
        self.assertFalse((caught.exception.output_dir / "score.json").exists())
        self.assert_source_preserved(original)

    def test_internal_transition_notes_are_preserved_beyond_the_planned_boundary(self):
        def transition(*args):
            raw, inference = self.fake_model(*args)
            if args[2].name == "passage-1":
                response = json.loads(raw)
                response["notes"].append(self.note("lead", 67, 27.5, 2, .5))
                return json.dumps(response), inference
            return raw, inference
        score = self.generate(transition)
        note = next(n for n in score['events'] if n['source_passage']==1 and n['midi']==67)
        self.assertEqual((note['start_beat'],note['length_beats']),(27.5,2))
        self.assertEqual(score['duration_seconds'],30.5)
        self.assertLessEqual(score['study']['continuity_evidence']['global_max_gap_seconds'],2)

    def test_failed_authoring_preserves_the_original_and_inspectable_draft(self):
        original = copy.deepcopy(self.source)
        def fail(*args):
            if args[2].name == "passage-1":
                raise RuntimeError("model stopped")
            return self.fake_model(*args)
        with self.assertRaises(OrganistError) as caught:
            self.generate(fail)
        self.assertTrue((caught.exception.output_dir / "draft-prefix.json").exists())
        self.assertFalse((caught.exception.output_dir / "score.json").exists())
        self.assert_source_preserved(original)

    def test_invalid_review_cannot_partially_replace_a_valid_draft(self):
        def bad_review(*args):
            raw, inference = self.fake_model(*args)
            if args[2].name == "model-review":
                response = json.loads(raw)
                response["replacements"][0]["notes"] = [self.note("lead", 70, 0, 1, .6)]
                return json.dumps(response), inference
            return raw, inference
        score = self.generate(bad_review)
        self.assertEqual(score["study"]["review_status"], "failed_draft_retained")
        self.assertEqual(score["study"]["revised_passages"], [])
        self.assertEqual(score["events"][0]["midi"], 64)
        directory = Path(score["study"]["output_dir"])
        self.assertEqual(json.loads((directory / "draft-score.json").read_text())["events"], score["events"])

    def test_same_notes_with_a_new_title_are_not_published(self):
        def identical(*args):
            raw, inference = self.fake_model(*args)
            response = json.loads(raw)
            name = args[2].name
            if name == "plan":
                response["passages"][0].update(start_beat=0, end_beat=.25)
                response["passages"][1].update(start_beat=.25, end_beat=.5)
            elif name == "passage-0":
                response["notes"] = [self.note("lead", 60, 0, .25, 1)]
            elif name == "passage-1":
                response["notes"] = [self.note("texture", 67, .25, .25, 1)]
            elif name == "model-review":
                response["replacements"] = []
            return json.dumps(response), inference
        with self.assertRaisesRegex(OrganistError, "repeats the source note ledger") as caught:
            self.generate(identical)
        self.assertFalse((caught.exception.output_dir / "score.json").exists())

    def test_reports_every_long_global_gap_without_modifying_notes(self):
        events = []
        for index, start in enumerate((0, 23.75, 44.825)):
            event = copy.deepcopy(self.source["events"][0])
            event.update(id=index, start_seconds=start, duration_seconds=1, start_beat=start * 2, length_beats=2)
            events.append(event)
        before = copy.deepcopy(events)
        with self.assertRaises(ValueError) as caught:
            _require_continuity(events, 120, 0, 91.65, "regression study")
        self.assertIn("beats 2..47.5 (22.75s)", str(caught.exception))
        self.assertIn("beats 49.5..89.65 (20.075s)", str(caught.exception))
        self.assertEqual(events, before)
        facts = score_facts(events, 120, [], end_beat=91.65)
        self.assertAlmostEqual(facts["global_max_gap_seconds"], 22.75)
        self.assertLess(facts["occupied_fraction"], .1)

    def test_polyphony_uses_renderer_frames_at_touching_float_boundaries(self):
        events = copy.deepcopy(self.source["events"])
        events[0].update(start_seconds=.1, duration_seconds=.2, start_beat=.1, length_beats=.2)
        events[1].update(start_seconds=.3, duration_seconds=.2, start_beat=.3, length_beats=.2)
        self.assertGreater(.1 + .2, .3)
        facts = score_facts(events, 60, [])
        self.assertEqual(facts["global_max_polyphony"], 1)
        self.assertEqual(facts["cross_part_overlap_beats"], 0)
        self.assertEqual(facts["analysis_sample_rate"], 48000)

    def test_activity_outside_analysis_window_does_not_inflate_window_facts(self):
        events = []
        for index, (part, start, length) in enumerate((("lead", 0, 4), ("texture", 1, 2),
                                                      ("lead", 10, 1), ("texture", 13, 2))):
            event = copy.deepcopy(next(e for e in self.source["events"] if e["layer"] == part))
            event.update(id=index, start_seconds=start, duration_seconds=length,
                         start_beat=start, length_beats=length)
            events.append(event)
        original = copy.deepcopy(events)
        facts = score_facts(events, 60, [], start_beat=10, end_beat=12)
        self.assertEqual(facts["global_max_polyphony"], 1)
        self.assertEqual(facts["occupied_beats"], 1)
        self.assertEqual(facts["cross_part_overlap_beats"], 0)
        self.assertEqual(facts["occupied_fraction"], .5)
        self.assertEqual(facts["parts"]["lead"]["occupied_beats"], 1)
        self.assertEqual(facts["parts"]["texture"]["occupied_beats"], 0)
        self.assertEqual(facts["parts"]["texture"]["max_simultaneous_notes"], 0)
        self.assertEqual(facts["silence_intervals"], [{"start_beat": 11, "end_beat": 12, "seconds": 1}])
        self.assertEqual(events, original)
        silent = score_facts(events, 60, [], start_beat=20, end_beat=22)
        self.assertEqual(silent["global_max_polyphony"], 0)
        self.assertEqual(silent["occupied_beats"], 0)
        self.assertEqual(silent["occupied_fraction"], 0)
        self.assertEqual(silent["global_max_gap_seconds"], 2)

    def test_notes_crossing_analysis_edges_are_measured_only_inside_window(self):
        events = copy.deepcopy(self.source["events"])
        events[0].update(start_seconds=0, duration_seconds=12, start_beat=0, length_beats=12)
        events[1].update(start_seconds=11, duration_seconds=20, start_beat=11, length_beats=20)
        facts = score_facts(events, 60, [], start_beat=10, end_beat=13)
        self.assertEqual(facts["global_max_polyphony"], 2)
        self.assertEqual(facts["occupied_beats"], 3)
        self.assertEqual(facts["cross_part_overlap_beats"], 1)
        self.assertEqual(facts["cross_part_overlap_fraction_of_occupied_time"], 1 / 3)
        self.assertEqual(facts["parts"]["lead"]["occupied_beats"], 2)
        self.assertEqual(facts["parts"]["texture"]["occupied_beats"], 2)
        self.assertEqual(facts["occupied_fraction"], 1)
        self.assertEqual(facts["silence_intervals"], [])

    def test_review_and_final_evidence_include_actual_notes_after_planned_end(self):
        def tail(*args):
            raw, inference = self.fake_model(*args)
            if args[2].name == "model-review":
                response = json.loads(raw)
                response["replacements"] = [{"passage_index": 1, "reason": "An authored ending after the planned boundary",
                                              "notes": [self.note("texture", 50, 12, 16, .3),
                                                        self.note("lead", 65, 29, 1, .5)]}]
                return json.dumps(response), inference
            return raw, inference
        with patch("flycomposer.organist_study._require_continuity", wraps=_require_continuity) as checked:
            score = self.generate(tail)
        self.assertEqual(score["study"]["review_status"], "completed")
        self.assertEqual(score["duration_seconds"], 31)
        evidence = score["study"]["continuity_evidence"]
        self.assertEqual(evidence["analysis_window_beats"], [0, 30])
        self.assertEqual(evidence["global_max_gap_seconds"], 1)
        self.assertIn({"start_beat": 28, "end_beat": 29, "seconds": 1}, evidence["silence_intervals"])
        joint = [call for call in checked.call_args_list if call.args[4] == "joint model-review candidate"]
        self.assertEqual(len(joint), 1)
        self.assertEqual(joint[0].args[3], 30)
        final = [call for call in checked.call_args_list if call.args[4] == "final combined study"]
        self.assertEqual(final[0].args[3], 30)

    def test_joint_review_cannot_create_a_gap_between_individually_valid_passages(self):
        def joint_gap(*args):
            raw, inference = self.fake_model(*args)
            response = json.loads(raw)
            name = args[2].name
            if name == "plan":
                response["passages"][1]["start_beat"] = 14
            elif name == "passage-1":
                response["notes"] = [self.note("texture", 50, 14, 14, .3)]
            elif name == "model-review":
                response["replacements"] = [
                    {"passage_index": 0, "reason": "End a little earlier", "notes": [self.note("texture", 52, 0, 12, .3)]},
                    {"passage_index": 1, "reason": "Enter a little later", "notes": [self.note("texture", 50, 16, 12, .3)]}]
            return json.dumps(response), inference
        score = self.generate(joint_gap)
        self.assertEqual(score["study"]["review_status"], "failed_draft_retained")
        self.assertEqual(score["study"]["revised_passages"], [])
        self.assertEqual(score["study"]["continuity_evidence"]["global_max_gap_seconds"], 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)

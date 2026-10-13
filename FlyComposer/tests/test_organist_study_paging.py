"""Small authoring requests retain model notes and whole-passage identity."""
import copy
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from flycomposer.organist import expand_authoring
from flycomposer.organist_continuation import _flat
from flycomposer.organist_study import generate_study
from flycomposer.render import assess_performance, render_score


class StudyPagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        for folder, name in (("data", "instruments.json"), ("profiles", "organist-study.md")):
            (self.root / folder).mkdir()
            shutil.copy2(ROOT / folder / name, self.root / folder / name)
        self.source = expand_authoring({
            "title": "Earlier take", "bpm": 120,
            "voices": {"lead": "classic:keys", "harmony": "classic:pad"},
            "clips": [{"part": "lead", "events": [[60, 0, 1, .5]], "placements": [[0, 0]]},
                      {"part": "harmony", "events": [[48, 0, 1, .3]], "placements": [[0, 0]]}]},
            {"classic:keys", "classic:pad"}, seed=2026, seconds=75)
        self.plan = {"title": "Connected authored pages", "idea": "Develop one subject across a long passage",
                     "part_intentions": {"lead": "Develop the subject", "harmony": "Carry its rests"},
                     "passages": [{"name": "Development", "start_beat": 0, "end_beat": 36, "intent": "Remember and develop"},
                                  {"name": "Arrival", "start_beat": 36, "end_beat": 48, "intent": "Resolve the developed phrase"}]}
        self.calls = []
        self.authored = {}
        self.bad_join = False

    def tearDown(self):
        self.temporary.cleanup()

    def fake_model(self, endpoint, payload, directory, attempt, progress):
        name = directory.name
        self.calls.append((name, attempt, copy.deepcopy(payload)))
        context = json.loads(payload["messages"][1]["content"].split("\n", 1)[1])
        if name == "plan":
            response = copy.deepcopy(self.plan)
        elif name == "model-review":
            self.review_context = context
            response = {"critique": "Keep the complete authored draft for the listener to judge.", "replacements": []}
        else:
            window = context["work_window"]
            start, end = window["start_beat"], window["end_beat"]
            # These fixture notes are the fake author's response, not a decoder
            # recipe: equality below detects any later filling or timing edits.
            notes = [
                {"part": "harmony", "midi": 48 + window["page_index"], "beat": start,
                 "length": end - start + (1 if window["page_count"] > 1 else 0), "velocity": .23},
                {"part": "lead", "midi": 67 - window["page_index"], "beat": start + .5, "length": 1.25, "velocity": .61},
                {"part": "lead", "midi": 64 + window["page_index"], "beat": end - 3, "length": 1.5, "velocity": .47}]
            if self.bad_join and name == "passage-0-page-0":
                notes = [{"part": "harmony", "midi": 48, "beat": 0, "length": 9, "velocity": .23}]
            elif self.bad_join and name == "passage-0-page-1" and attempt == 0:
                notes = [{"part": "harmony", "midi": 50, "beat": 15, "length": 9, "velocity": .23}]
            response = {"title": "Authored page", "notes": notes}
            self.authored[name] = copy.deepcopy(notes)
        (directory / f"request-{attempt}.json").write_text(json.dumps(payload), encoding="utf-8")
        (directory / f"authoring-{attempt}.txt").write_text(json.dumps(response), encoding="utf-8")
        return json.dumps(response), {"model": "isolated-test-author", "done_reason": "stop"}

    def generate(self):
        with patch("flycomposer.organist_study._stream_request", side_effect=self.fake_model), \
             patch.dict("os.environ", {"TELEDRA_OLLAMA_URL": "http://localhost:11434", "TELEDRA_MODEL": "isolated-test-author"}):
            return generate_study(self.root, self.source, "Keep this exact feedback.\nDevelop the subject.", seconds=75)

    def test_pages_preserve_authored_notes_context_overlap_and_original_passages(self):
        source_before = copy.deepcopy(self.source)
        score = self.generate()
        self.assertEqual([name for name, _, _ in self.calls],
                         ["plan", "passage-0-page-0", "passage-0-page-1", "passage-0-page-2", "passage-1", "model-review"])
        self.assertEqual(score["plan"], self.plan)
        self.assertEqual(self.source, source_before)
        self.assertEqual(len(score["sections"]), 2)
        windows = score["inference_provenance"]["authoring_windows"]
        self.assertEqual(len(windows), 4)
        prior_page_notes = []
        for window in windows:
            self.assertLessEqual(window["end_beat"] - window["start_beat"], 16)
            self.assertEqual(window["status"], "accepted")
            phase_name = window["phase"]
            events = [event for event in score["events"] if event["source_phase"] == phase_name]
            self.assertEqual([_flat(event) for event in events], self.authored[phase_name])
            self.assertEqual([event["source_event"] for event in events], list(range(len(events))))
            self.assertTrue(all(event["source_passage"] == window["source_passage"] and event["source_page"] == window["page_index"] for event in events))
            payload = next(payload for name, _, payload in self.calls if name == phase_name)
            prompt = payload["messages"][1]["content"]
            context = json.loads(prompt.split("\n", 1)[1])
            self.assertEqual(context["your_plan"], self.plan)
            self.assertEqual(context["passage"], self.plan["passages"][window["source_passage"]])
            self.assertEqual(context["operator_feedback"], "Keep this exact feedback.\nDevelop the subject.")
            self.assertIn("WHOLE work window first", prompt)
            self.assertIn("quarter-note beats", prompt)
            self.assertIn(f"absolute beat {window['required_coverage_end_beat']:g}", prompt)
            self.assertEqual(window["start_seconds"], window["start_beat"] / 2)
            if window["source_passage"] == 0:
                self.assertEqual(context["earlier_pages_in_this_passage"], prior_page_notes)
                for note in prior_page_notes:
                    self.assertIn(note, context["previous_new_study_notes"])
                prior_page_notes += self.authored[phase_name]
        self.assertEqual([len(item["notes"]) for item in self.review_context["passages"]], [9, 3])
        self.assertEqual([item["passage_index"] for item in self.review_context["passages"]], [0, 1])
        self.assertEqual(len({note["id"] for note in score["events"]}), len(score["events"]))
        self.assertEqual(score["events"][0]["length_beats"], 13)  # Authored one-beat page overhang survives.
        self.assertEqual(score["study"]["continuity_evidence"]["global_max_gap_seconds"], 0)
        assess_performance(score)
        _, _, audit = render_score(score, note_factory=lambda note, rate: np.ones(int(note["duration_seconds"] * rate)), sr=800)
        self.assertTrue(audit["complete"])
        self.assertEqual(audit["notes_mixed"], 12)
        prefix = json.loads((Path(score["study"]["output_dir"]) / "draft-prefix.json").read_text())
        self.assertEqual(prefix["events"], score["events"])

    def test_join_gap_requires_model_repair_without_filling_previous_page(self):
        self.bad_join = True
        score = self.generate()
        attempts = [payload for name, _, payload in self.calls if name == "passage-0-page-1"]
        self.assertEqual(len(attempts), 2)
        self.assertIn("combined study so far", attempts[1]["messages"][-1]["content"])
        self.assertIn("beats 9..15 (3s)", attempts[1]["messages"][-1]["content"])
        first = [event for event in score["events"] if event["source_phase"] == "passage-0-page-0"]
        self.assertEqual([_flat(event) for event in first], self.authored["passage-0-page-0"])
        self.assertEqual(first[0]["length_beats"], 9)
        self.assertEqual(score["study"]["continuity_evidence"]["global_max_gap_seconds"], 1.5)
        self.assertTrue(all(event["start_beat"] < 9 or event["start_beat"] >= 12 for event in score["events"]))


if __name__ == "__main__":
    unittest.main()

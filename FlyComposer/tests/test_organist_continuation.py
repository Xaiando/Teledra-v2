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
from flycomposer.organist import expand_authoring
from flycomposer.organist_continuation import PROTOCOL, _activity, _expand_notes, _flat, generate_continuation
from flycomposer.render import assess_performance, note_seed


class ContinuationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "data").mkdir()
        (self.root / "profiles").mkdir()
        shutil.copy2(ROOT / "data/instruments.json", self.root / "data/instruments.json")
        (self.root / "profiles/organist.md").write_text("The authored Organist persona.", encoding="utf-8")
        authoring = {"title": "Accepted music", "bpm": 60, "voices": {"lead": "classic:keys"},
                     "clips": [{"part": "lead", "events": [[60, 0, 2, .6], [64, 6, 1, .4]], "placements": [[0, 0]]}]}
        self.source = expand_authoring(authoring, {"classic:keys"}, seed=123, seconds=75)
        self.source["inference_provenance"] = {"model": "original-model", "profile_sha256": "prior-profile"}
        self.requests = []

    def tearDown(self):
        self.tmp.cleanup()

    def fake_model(self, endpoint, payload, output_dir, attempt, progress):
        self.requests.append(copy.deepcopy(payload))
        # Transport saving itself is tested in test_organist against fake HTTP.
        (output_dir / f"request-{attempt}.json").write_text(json.dumps(payload), encoding="utf-8")
        context = json.loads(payload["messages"][1]["content"].split("\n", 1)[1])
        start = context["next_note_beat_min"]
        end = context["next_note_end_beat_max"]
        authored = {"title": "The model's next movement", "notes": [
            {"part": "lead", "midi": 67, "beat": start, "length": 1, "velocity": .35},
            {"part": "lead", "midi": 62, "beat": start + 1, "length": end - start - 1, "velocity": .55}]}
        return json.dumps(authored), {"model": "actual-test-model", "eval_count": 100, "done_reason": "stop"}

    def generate(self, model=None, **kwargs):
        with patch("flycomposer.organist_continuation._stream_request", side_effect=model or self.fake_model), \
             patch.dict("os.environ", {"TELEDRA_OLLAMA_URL": "http://localhost:11434", "TELEDRA_MODEL": "configured-test-model"}):
            return generate_continuation(self.root, self.source, **kwargs)

    def test_reaches_target_through_real_authored_windows_and_preserves_prefix(self):
        original = copy.deepcopy(self.source)
        score = self.generate(seconds=120)
        self.assertEqual(self.source, original)
        self.assertEqual(score["events"][:len(original["events"])], original["events"])
        self.assertEqual(score["seed"], original["seed"])
        self.assertEqual(score["voices"], original["voices"])
        self.assertEqual(score["bpm"], original["bpm"])
        for event in original["events"]:
            self.assertEqual(note_seed(original, event), note_seed(score, event))
        self.assertEqual(score["duration_seconds"], 120)
        self.assertEqual(score["continuation"]["status"], "complete")
        self.assertTrue(score["continuation"]["target_reached"])
        self.assertEqual(score["continuation"]["new_chunks"], 6)
        self.assertEqual(score["continuation"]["new_notes"], 12)
        self.assertEqual(len(self.requests), 6)
        self.assertEqual(score["source_inference_provenance"], original["inference_provenance"])
        self.assertEqual(score["inference_provenance"]["returned_models"], ["actual-test-model"])
        assess_performance(score)
        for receipt in score["continuation_authoring"]:
            notes = [event for event in score["events"] if event.get("source_chunk") == receipt["chunk"]]
            self.assertEqual(notes[0]["start_seconds"], receipt["first_allowed_beat"])
            self.assertEqual(notes[0]["midi"], 67)
            self.assertEqual(notes[0]["velocity"], .35)
            self.assertEqual(notes[1]["start_seconds"], receipt["first_allowed_beat"] + 1)
        directory = Path(score["continuation"]["output_dir"])
        self.assertEqual(json.loads((directory / "source-score.json").read_text()), original)
        self.assertEqual(json.loads((directory / "accepted-prefix.json").read_text())["events"], score["events"])
        contexts = [json.loads((directory / f"chunk-{i:02d}" / "context.json").read_text()) for i in range(6)]
        self.assertEqual(contexts[0]["source_motif_examples"]["lead"], [_flat(e) for e in original["events"]])
        self.assertEqual(contexts[1]["previous_chunk_performed_notes"], score["continuation_authoring"][0]["authoring"]["notes"])
        self.assertEqual(contexts[1]["playhead_beat"], 26)
        self.assertEqual(contexts[1]["next_note_beat_min"], 25)
        self.assertNotIn('"clips"', json.dumps(contexts))
        self.assertEqual(score["arrangement_version"], PROTOCOL)
        note_schema = self.requests[0]["format"]["properties"]["notes"]
        self.assertEqual(note_schema["maxItems"], 64)
        self.assertEqual(note_schema["items"]["properties"]["beat"]["minimum"], 6)

    def test_late_transport_failure_returns_valid_prefix_with_honest_duration(self):
        count = 0
        def fail_later(*args):
            nonlocal count
            count += 1
            if count == 3:
                raise RuntimeError("model unavailable")
            return self.fake_model(*args)
        score = self.generate(fail_later, seconds=120)
        self.assertEqual(score["continuation"]["status"], "partial")
        self.assertFalse(score["continuation"]["target_reached"])
        self.assertEqual(score["duration_seconds"], 46)
        self.assertEqual(score["continuation"]["new_notes"], 4)
        self.assertEqual(score["continuation"]["accepted_chunks"], 2)
        self.assertIn("model unavailable", score["continuation"]["stop_reason"])
        self.assertEqual(score["events"][:2], self.source["events"])
        assess_performance(score)

    def test_first_failure_returns_no_duplicate_new_music(self):
        def fail(*args):
            raise RuntimeError("no model")
        score = self.generate(fail, seconds=120)
        self.assertEqual(score["events"], self.source["events"])
        self.assertEqual(score["continuation"]["new_notes"], 0)
        self.assertEqual(score["continuation"]["status"], "partial")
        self.assertTrue(Path(score["continuation"]["output_dir"], "accepted-prefix.json").exists())

    def test_one_factual_repair_for_invalid_absolute_timing(self):
        count = 0
        def repair(*args):
            nonlocal count
            count += 1
            raw, facts = self.fake_model(*args)
            if count == 1:
                authored = json.loads(raw)
                authored["notes"][0]["beat"] = 500
                return json.dumps(authored), facts
            return raw, facts
        score = self.generate(repair, seconds=40)
        self.assertEqual(score["continuation"]["status"], "complete")
        self.assertEqual(len(self.requests), 3)
        repair_prompt = self.requests[1]["messages"][-1]["content"]
        self.assertIn("Factual sequencer validation error", repair_prompt)
        self.assertEqual(score["events"][:2], self.source["events"])
        attempts = score["inference_provenance"]["chunks"][0]["attempts"]
        self.assertEqual([attempt["status"] for attempt in attempts], ["invalid_authoring", "accepted_authoring"])

    def test_insufficient_progress_gets_one_repair_then_stops_without_padding(self):
        def tiny(*args):
            raw, facts = self.fake_model(*args)
            authored = json.loads(raw)
            authored["notes"] = [dict(authored["notes"][0], length=.5)]
            return json.dumps(authored), facts
        score = self.generate(tiny, seconds=120)
        self.assertEqual(len(self.requests), 2)
        self.assertEqual(score["continuation"]["new_notes"], 0)
        self.assertEqual(score["duration_seconds"], self.source["duration_seconds"])
        self.assertIn("extends the accepted timeline", score["continuation"]["stop_reason"])

    def test_tempo_or_voice_changes_never_rewrite_the_source(self):
        for changed in ("tempo", "voice"):
            self.requests = []
            def wrong(*args):
                raw, facts = self.fake_model(*args)
                authored = json.loads(raw)
                if changed == "tempo":
                    authored["bpm"] = 120
                else:
                    authored["notes"][0]["part"] = "unsupported"
                return json.dumps(authored), facts
            with self.subTest(changed=changed):
                score = self.generate(wrong, seconds=120)
                self.assertEqual(score["events"], self.source["events"])
                self.assertEqual(score["continuation"]["new_notes"], 0)
                self.assertEqual(len(self.requests), 2)

    def test_request_bound_stops_a_valid_progressing_prefix(self):
        with patch("flycomposer.organist_continuation.MAX_REQUESTS", 2):
            score = self.generate(seconds=120)
        self.assertEqual(len(self.requests), 2)
        self.assertEqual(score["continuation"]["stop_reason"], "request_limit")
        self.assertEqual(score["continuation"]["status"], "partial")
        self.assertEqual(score["duration_seconds"], 46)

    def test_density_summary_counts_overlaps_without_changing_music(self):
        notes = copy.deepcopy(self.source["events"])
        notes[1]["start_seconds"] = .5
        notes[1]["duration_seconds"] = 1
        before = copy.deepcopy(notes)
        facts = _activity(notes)
        self.assertEqual(facts["parts"]["lead"]["max_simultaneous_notes"], 2)
        self.assertEqual(notes, before)

    def test_all_window_overruns_are_reported_in_one_factual_error(self):
        authored = {"title": "Window arithmetic", "notes": [
            {"part": "lead", "midi": 60, "beat": 8, "length": 1, "velocity": .5},
            {"part": "lead", "midi": 64, "beat": 25, "length": 2, "velocity": .5},
            {"part": "lead", "midi": 67, "beat": 24, "length": 4, "velocity": .5}]}
        before = copy.deepcopy(authored)
        with self.assertRaises(ValueError) as caught:
            _expand_notes(authored, self.source["voices"], 60, 6, 26, 7, 6000)
        message = str(caught.exception)
        self.assertIn("n1:start=25,end=27", message)
        self.assertIn("n2:start=24,end=28", message)
        self.assertIn("<= 26", message)
        self.assertEqual(authored, before)

    def test_continuing_a_partial_session_retains_prior_authored_context(self):
        with patch("flycomposer.organist_continuation.MAX_REQUESTS", 1):
            previous = self.generate(seconds=120)
        prior_receipts = copy.deepcopy(previous["continuation_authoring"])
        self.source = previous
        self.requests = []
        with patch("flycomposer.organist_continuation.MAX_REQUESTS", 1):
            next_score = self.generate(seconds=120)
        context = json.loads(self.requests[0]["messages"][1]["content"].split("\n", 1)[1])
        self.assertEqual(context["previous_chunk_performed_notes"], prior_receipts[-1]["authoring"]["notes"])
        self.assertEqual(context["prior_session_chunks"], 1)
        self.assertEqual(context["previous_chunks"][0]["end_beat"], prior_receipts[0]["end_seconds"])
        self.assertEqual(next_score["continuation_authoring"][:1], prior_receipts)
        self.assertEqual(len(next_score["continuation_authoring"]), 2)
        self.assertEqual(next_score["continuation"]["new_chunks"], 1)
        self.assertEqual(next_score["events"][:len(previous["events"])], previous["events"])

    def test_absolute_beats_convert_once_at_nonunit_tempo(self):
        authored = {"title": "Absolute clock", "notes": [
            {"part": "lead", "midi": 65, "beat": 87.75, "length": 12, "velocity": .7}]}
        notes = _expand_notes(authored, self.source["voices"], 120, 87.75, 127.75, 44.875, 6000)
        self.assertEqual(notes[0]["start_beat"], 87.75)
        self.assertEqual(notes[0]["start_seconds"], 43.875)
        self.assertEqual(notes[0]["duration_seconds"], 6)
        self.assertEqual(notes[0]["midi"], 65)

    def test_legacy_receipts_stay_on_disk_but_context_has_only_absolute_notes(self):
        legacy = {"chunk": 0, "anchor_seconds": 0, "end_seconds": 7, "notes": 2,
                  "authoring": copy.deepcopy(self.source["authoring"])}
        self.source["continuation_authoring"] = [legacy]
        with patch("flycomposer.organist_continuation.MAX_REQUESTS", 1):
            result = self.generate(seconds=120)
        context = json.loads(self.requests[0]["messages"][1]["content"].split("\n", 1)[1])
        self.assertEqual(context["previous_chunk_performed_notes"], [_flat(e) for e in self.source["events"]])
        self.assertNotIn('"clips"', json.dumps(context))
        self.assertNotIn("anchor_seconds", json.dumps(context))
        self.assertEqual(result["continuation_authoring"][0], legacy)


if __name__ == "__main__":
    unittest.main(verbosity=2)

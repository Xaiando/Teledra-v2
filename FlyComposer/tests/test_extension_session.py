"""Continuation/reuse transactions with fake authors and isolated WAV exports."""
import copy
import hashlib
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
from flycomposer.experiment import Studio, atomic_json
from flycomposer.render import render_score


def rehash(score):
    score["event_sha256"] = hashlib.sha256(json.dumps(score["events"], sort_keys=True).encode()).hexdigest()
    return score


def source_score(performer, duration):
    events = [{"id": i, "layer": "lead", "voice": "classic:keys", "midi": pitch,
               "start_beat": start, "length_beats": .5, "velocity": .5,
               "start_seconds": start, "duration_seconds": .5, "section": 0}
              for i, (pitch, start) in enumerate(((60, 0), (67, duration - 3)))]
    return rehash({"kind": "performance", "performer": performer, "condition": performer,
                   "seed": 2026, "bpm": 60, "title": performer + " original", "voices": {"lead": "classic:keys"},
                   "duration_seconds": duration, "events": events,
                   "sections": [{"name": "Recording", "start_seconds": 0, "end_seconds": duration}]})


def continued_score(source, duration, target=120):
    result = copy.deepcopy(source)
    result["events"].append({"id": max(e["id"] for e in source["events"]) + 1,
                              "layer": "lead", "voice": "classic:keys", "midi": 64,
                              "start_beat": duration - 2, "length_beats": .5, "velocity": .4,
                              "start_seconds": duration - 2, "duration_seconds": .5, "section": 0})
    result.update(title=source["title"] + " continued", duration_seconds=duration,
                  sections=[{"name": "Extended recording", "start_seconds": 0, "end_seconds": duration}],
                  continuation={"status": "complete" if duration >= target - 2 else "partial",
                                "target_seconds": target, "target_reached": duration >= target - 2,
                                "actual_seconds": duration, "new_notes": 1})
    return rehash(result)


def isolated_render(score, progress):
    return render_score(score, note_factory=lambda e, sr: 1 + np.arange(int(e["duration_seconds"] * sr)) % 7,
                        sr=400, progress=progress)


class ExtensionSessionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        (self.root / "data").mkdir()
        for name in ("circuit.npz", "circuit.json", "instruments.json"):
            shutil.copy2(ROOT / "data" / name, self.root / "data" / name)
        self.studio = Studio(self.root)
        with patch("flycomposer.autonomous.write_performance", return_value=source_score("fly", 75)), \
             patch("flycomposer.organist.generate_performance", return_value=source_score("organist", 10)), \
             patch("flycomposer.experiment.render_score", side_effect=isolated_render):
            self.original = self.studio.perform(seed=2026, seconds=75)
        self.original_tracks = {t["performer"]: copy.deepcopy(t) for t in self.studio.library["tracks"]}
        self.original_files = {t["id"]: self.files(t["id"]) for t in self.original_tracks.values()}

    def tearDown(self):
        self.temporary.cleanup()

    def files(self, track_id):
        return {path.name: path.read_bytes() for path in (self.root / "library" / track_id).iterdir()}

    def score(self, track_id):
        return json.loads((self.root / "library" / track_id / "score.json").read_text())

    def assert_original_files_unchanged(self):
        for track_id, files in self.original_files.items():
            self.assertEqual(self.files(track_id), files)

    def make_partial(self):
        with patch("flycomposer.autonomous.extend_performance", side_effect=lambda c, s, seconds: continued_score(s, 120)), \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=lambda r, s, seconds, progress: continued_score(s, 44)), \
             patch("flycomposer.experiment.render_score", side_effect=isolated_render):
            return self.studio.perform(source_id=self.original["id"], seconds=120)

    def test_continuation_preserves_every_source_note_file_and_concurrent_feedback(self):
        captured = {}
        def fly(circuit, source, seconds):
            captured["fly"] = copy.deepcopy(source)
            return continued_score(source, seconds)
        def organist(root, source, seconds, progress):
            captured["organist"] = copy.deepcopy(source)
            old_id = self.original_tracks["organist"]["id"]
            self.studio.feedback({"track_id": old_id, "verdict": "keep", "note": "Keep the short original"})
            published = self.studio.library["comparisons"][-1]
            self.assertNotEqual(published["id"], self.original["id"])
            self.studio.comparison_action({"comparison_id": published["id"], "note": "Written while Organist continued",
                                          "preferred_track_id": published["track_ids"][0]})
            return continued_score(source, 44)
        with patch("flycomposer.autonomous.extend_performance", side_effect=fly) as fly_author, \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=organist) as model, \
             patch("flycomposer.autonomous.write_performance", side_effect=AssertionError("Restarted Fly")), \
             patch("flycomposer.organist.generate_performance", side_effect=AssertionError("Restarted Organist")), \
             patch("flycomposer.experiment.render_score", side_effect=isolated_render):
            batch = self.studio.perform(seed=999, source_id=self.original["id"], seconds=120)
        fly_author.assert_called_once()
        model.assert_called_once()
        self.assertEqual(batch["seed"], 2026)
        self.assertEqual(batch["source_comparison_id"], self.original["id"])
        self.assertEqual(batch["continuation_status"]["organist"]["status"], "partial")
        self.assertEqual(batch["feedback"]["note"], "Written while Organist continued")
        saved = Studio(self.root)
        self.assertEqual(len(saved.library["tracks"]), 4)
        self.assertEqual(len(saved.state()["library"]), 4)
        for track in saved.library["tracks"][2:]:
            original = captured[track["performer"]]
            result = self.score(track["id"])
            by_id = {event["id"]: event for event in result["events"]}
            self.assertTrue(all(by_id[event["id"]] == event for event in original["events"]))
            self.assertEqual(track["source_track_id"], self.original_tracks[track["performer"]]["id"])
            self.assertIsNone(track["verdict"])
        self.assertEqual(saved.library["tracks"][1]["note"], "Keep the short original")
        self.assert_original_files_unchanged()

    def test_continue_partial_pair_reuses_completed_audio_without_duplicate_track(self):
        partial = self.make_partial()
        fly_id = next(i for i in partial["track_ids"] if self.score(i)["performer"] == "fly")
        organist_id = next(i for i in partial["track_ids"] if self.score(i)["performer"] == "organist")
        completed_files = self.files(fly_id)
        def organist(root, source, seconds, progress):
            self.assertEqual(source, self.score(organist_id))
            self.studio.feedback({"track_id": fly_id, "verdict": "keep", "note": "This complete Fly remains"})
            current = self.studio.library["comparisons"][-1]
            self.studio.comparison_action({"comparison_id": current["id"], "note": "Continue only Organist",
                                          "preferred_track_id": fly_id})
            return continued_score(source, 120)
        with patch("flycomposer.autonomous.extend_performance", side_effect=AssertionError("Completed Fly rendered again")) as fly, \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=organist) as model, \
             patch("flycomposer.experiment.render_score", side_effect=isolated_render) as render:
            batch = self.studio.perform(source_id=partial["id"], seconds=120)
        fly.assert_not_called()
        model.assert_called_once()
        render.assert_called_once()
        self.assertEqual(batch["track_ids"][0], fly_id)
        self.assertTrue(batch["continuation_status"]["fly"]["reused"])
        self.assertEqual(batch["feedback"]["note"], "Continue only Organist")
        self.assertEqual(self.files(fly_id), completed_files)
        saved = Studio(self.root)
        self.assertEqual(len(saved.library["tracks"]), 5)
        self.assertEqual(len(saved.state()["library"]), 5)
        self.assertEqual(sum(t["id"] == fly_id for t in saved.library["tracks"]), 1)
        self.assertEqual(next(t for t in saved.library["tracks"] if t["id"] == fly_id)["note"], "This complete Fly remains")
        self.assert_original_files_unchanged()

    def test_reused_recording_survives_failure_and_retry_of_only_missing_author(self):
        partial = self.make_partial()
        fly_id = next(i for i in partial["track_ids"] if self.score(i)["performer"] == "fly")
        original_fly_files = self.files(fly_id)
        with patch("flycomposer.autonomous.extend_performance", side_effect=AssertionError("Completed Fly restarted")) as fly, \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=ValueError("Continuation failed")):
            failed = self.studio.perform(source_id=partial["id"], seconds=120)
        self.assertEqual(failed["track_ids"], [fly_id])
        self.assertEqual(failed["performer_status"], {"fly": "recorded", "organist": "failed"})
        fly.assert_not_called()
        with patch("flycomposer.autonomous.extend_performance", side_effect=AssertionError("Completed Fly restarted")) as fly, \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=lambda r, s, seconds, progress: continued_score(s, 120)), \
             patch("flycomposer.experiment.render_score", side_effect=isolated_render):
            retry = self.studio.perform(retry_id=failed["id"])
        fly.assert_not_called()
        self.assertEqual(retry["track_ids"][0], fly_id)
        self.assertEqual(len(retry["track_ids"]), 2)
        self.assertEqual(retry["retry_history"][0]["recording_errors"], failed["recording_errors"])
        self.assertEqual(self.files(fly_id), original_fly_files)
        self.assert_original_files_unchanged()

    def test_changed_source_identity_refuses_retry_before_authoring(self):
        with patch("flycomposer.autonomous.extend_performance", side_effect=lambda c, s, seconds: continued_score(s, 120)), \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=ValueError("Model unavailable")), \
             patch("flycomposer.experiment.render_score", side_effect=isolated_render):
            failed = self.studio.perform(source_id=self.original["id"], seconds=120)
        source_file = self.root / "library" / self.original_tracks["organist"]["id"] / "score.json"
        altered = json.loads(source_file.read_text())
        altered["title"] = "A different serialized source"
        atomic_json(source_file, altered)
        before = self.studio.index.read_bytes()
        with patch("flycomposer.autonomous.extend_performance") as fly, \
             patch("flycomposer.organist_continuation.generate_continuation") as model:
            with self.assertRaisesRegex(ValueError, "source score changed"):
                self.studio.start_performance(retry_id=failed["id"])
        fly.assert_not_called()
        model.assert_not_called()
        self.assertFalse(self.studio.busy)
        self.assertEqual(self.studio.index.read_bytes(), before)

    def test_changed_seed_tempo_or_prefix_cannot_be_published_as_continuation(self):
        for mutation in ("seed", "bpm", "note"):
            def altered(circuit, source, seconds):
                result = continued_score(source, seconds)
                if mutation == "note":
                    result["events"][0]["midi"] += 1
                else:
                    result[mutation] += 1
                return rehash(result)
            with self.subTest(mutation=mutation), \
                 patch("flycomposer.autonomous.extend_performance", side_effect=altered), \
                 patch("flycomposer.organist_continuation.generate_continuation", side_effect=ValueError("Stopped test Organist")), \
                 patch("flycomposer.experiment.render_score", side_effect=AssertionError("Invalid prefix was rendered")) as render:
                batch = self.studio.perform(source_id=self.original["id"], seconds=120)
            render.assert_not_called()
            self.assertEqual(batch["track_ids"], [])
            self.assertEqual(batch["performer_status"]["fly"], "failed")
            self.assertIn("Continuation changed", batch["recording_errors"][0])
        self.assert_original_files_unchanged()

    def test_completed_pair_and_missing_reuse_audio_are_refused_before_work(self):
        with patch("flycomposer.autonomous.extend_performance", side_effect=lambda c, s, seconds: continued_score(s, 120)), \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=lambda r, s, seconds, progress: continued_score(s, 120)), \
             patch("flycomposer.experiment.render_score", side_effect=isolated_render):
            complete = self.studio.perform(source_id=self.original["id"], seconds=120)
        with self.assertRaisesRegex(ValueError, "already reached"):
            self.studio.start_performance(source_id=complete["id"], seconds=120)
        partial = self.make_partial()
        fly_id = next(i for i in partial["track_ids"] if self.score(i)["performer"] == "fly")
        # Only remove the synthetic completed capture within this test's temp root.
        (self.root / "library" / fly_id / "audio.wav").unlink()
        before = self.studio.index.read_bytes()
        with patch("flycomposer.autonomous.extend_performance") as fly, \
             patch("flycomposer.organist_continuation.generate_continuation") as model:
            with self.assertRaises(ValueError):
                self.studio.start_performance(source_id=partial["id"], seconds=120)
        fly.assert_not_called()
        model.assert_not_called()
        self.assertFalse(self.studio.busy)
        self.assertEqual(self.studio.index.read_bytes(), before)


if __name__ == "__main__":
    unittest.main(verbosity=2)

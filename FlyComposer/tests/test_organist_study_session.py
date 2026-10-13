"""Organist study transactions with fake authoring and isolated real WAV output.

No model requests, primary-library writes or audio-device playback occur here.
"""
import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from flycomposer.experiment import Studio, atomic_json
from flycomposer.render import render_score


FEEDBACK = "  Keep the warmer voice.\nGive the answers room; develop the phrase beyond a scale.  "


def rehash(score):
    score["event_sha256"] = hashlib.sha256(json.dumps(score["events"], sort_keys=True).encode()).hexdigest()
    return score


def fixture_score(performer, seconds=30):
    events = [{"id": index, "layer": "lead", "voice": "classic:keys", "midi": pitch,
               "start_beat": start, "length_beats": .5, "velocity": .5,
               "start_seconds": start, "duration_seconds": .5, "section": 0}
              for index, (pitch, start) in enumerate(((60, 0), (67, seconds - 3)))]
    return rehash({"kind": "performance", "performer": performer, "condition": performer,
                   "seed": 2026, "bpm": 60, "title": performer + " original", "voices": {"lead": "classic:keys"},
                   "duration_seconds": seconds, "events": events,
                   "sections": [{"name": "Recording", "start_seconds": 0, "end_seconds": seconds}]})


def studied_score(source, seconds=75):
    """A fresh musical answer deliberately changes every source-note gesture."""
    result = copy.deepcopy(source)
    result.update(title="Organist study", duration_seconds=seconds,
                  sections=[{"name": "Study recording", "start_seconds": 0, "end_seconds": seconds}])
    result["events"] = [{"id": index, "layer": "lead", "voice": source["voices"]["lead"], "midi": pitch,
                         "start_beat": start * source["bpm"] / 60, "length_beats": .75 * source["bpm"] / 60,
                         "velocity": .45, "start_seconds": start, "duration_seconds": .75, "section": 0}
                        for index, (pitch, start) in enumerate(((64, .5), (69, seconds / 2), (62, seconds - 2)))]
    return rehash(result)


def device_free_render(score, progress):
    return render_score(score, note_factory=lambda event, rate: 1. + (np.arange(int(event["duration_seconds"] * rate)) + event["midi"]) % 7,
                        sr=400, progress=progress)


class OrganistStudySessionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        (self.root / "data").mkdir()
        for name in ("circuit.npz", "circuit.json", "instruments.json"):
            shutil.copy2(ROOT / "data" / name, self.root / "data" / name)
        self.studio = Studio(self.root)
        with patch("flycomposer.autonomous.write_performance", return_value=fixture_score("fly", 75)), \
             patch("flycomposer.organist.generate_performance", return_value=fixture_score("organist", 30)), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render):
            self.original = self.studio.perform(seed=2026, seconds=75)
        self.tracks = {track["performer"]: copy.deepcopy(track) for track in self.studio.library["tracks"]}
        self.studio.feedback({"track_id": self.tracks["organist"]["id"], "verdict": "keep", "note": "Retain this original opinion"})
        self.studio.comparison_action({"comparison_id": self.original["id"], "note": "The original comparison opinion", "motion": 4})
        self.original_records = copy.deepcopy(self.studio.library)
        self.original_files = {track["id"]: self.files(track["id"]) for track in self.tracks.values()}

    def tearDown(self):
        self.temporary.cleanup()

    def files(self, track_id):
        return {file.name: file.read_bytes() for file in (self.root / "library" / track_id).iterdir()}

    def source_score(self):
        return json.loads((self.root / "library" / self.tracks["organist"]["id"] / "score.json").read_text())

    def assert_source_files_unchanged(self):
        for track_id, before in self.original_files.items():
            self.assertEqual(self.files(track_id), before)

    def assert_no_fake_organist(self, batch):
        self.assertEqual(batch["track_ids"], [self.tracks["fly"]["id"]])
        self.assertEqual(batch["performer_status"], {"fly": "recorded", "organist": "failed"})
        saved = Studio(self.root)
        self.assertEqual(len(saved.library["tracks"]), 2)
        self.assertEqual(saved.library["tracks"], self.original_records["tracks"])
        self.assertEqual(saved.library["comparisons"][0], self.original_records["comparisons"][0])
        self.assert_source_files_unchanged()
        self.assertFalse(self.studio.busy)

    def test_study_reuses_fly_and_publishes_fresh_organist_with_literal_feedback(self):
        called = {}
        def author(root, source, feedback, seconds, progress):
            called.update(root=root, source=copy.deepcopy(source), feedback=feedback, seconds=seconds)
            progress("Synthetic study author is working")
            return studied_score(source, seconds)
        with patch("flycomposer.organist_study.generate_study", side_effect=author) as model, \
             patch("flycomposer.autonomous.write_performance", side_effect=AssertionError("Fly regenerated")) as fly, \
             patch("flycomposer.autonomous.extend_performance", side_effect=AssertionError("Fly extended")) as fly_extend, \
             patch("flycomposer.organist.generate_performance", side_effect=AssertionError("Source-free fallback used")), \
             patch("flycomposer.organist_continuation.generate_continuation", side_effect=AssertionError("Study became continuation")), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render) as render:
            batch = self.studio.study_organist(self.original["id"], FEEDBACK, seconds=75)
        model.assert_called_once(); render.assert_called_once(); fly.assert_not_called(); fly_extend.assert_not_called()
        self.assertEqual(called, {"root": self.root, "source": self.source_score(), "feedback": FEEDBACK, "seconds": 75})
        self.assertNotEqual(batch["id"], self.original["id"])
        self.assertEqual((batch["kind"], batch["performance_mode"], batch["seed"]), ("performance", "study", 2026))
        self.assertEqual(batch["performer_status"], {"fly": "recorded", "organist": "recorded"})
        self.assertEqual(batch["track_ids"][0], self.tracks["fly"]["id"])
        self.assertEqual(len(batch["track_ids"]), 2)
        context = batch["study_context"]
        self.assertEqual(context["feedback"], FEEDBACK)
        self.assertEqual(context["source_comparison_id"], self.original["id"])
        self.assertEqual(context["source_track_ids"], {role: track["id"] for role, track in self.tracks.items()})
        for role, track in self.tracks.items():
            self.assertEqual(context["source_score_sha256"][role], hashlib.sha256(self.original_files[track["id"]]["score.json"]).hexdigest())
        saved = Studio(self.root)
        self.assertEqual(saved.library["tracks"][:2], self.original_records["tracks"])
        self.assertEqual(saved.library["comparisons"][0], self.original_records["comparisons"][0])
        new_track = next(track for track in saved.library["tracks"] if track["id"] == batch["track_ids"][1])
        self.assertEqual((new_track["performer"], new_track["performance_mode"]), ("organist", "study"))
        self.assertEqual(new_track["source_track_id"], self.tracks["organist"]["id"])
        self.assertIsNone(new_track["verdict"])
        score = json.loads((self.root / "library" / new_track["id"] / "score.json").read_text())
        self.assertEqual(score["study_context"], context)
        for key in ("seed", "bpm", "voices"):
            self.assertEqual(score[key], self.source_score()[key])
        self.assertNotEqual(score["events"][0], self.source_score()["events"][0])
        self.assert_source_files_unchanged()
        self.assertFalse(self.studio.busy)

    def test_feedback_entered_while_studying_survives_successful_publication(self):
        def author(root, source, feedback, seconds, progress):
            current = self.studio.library["comparisons"][-1]
            self.assertNotEqual(current["id"], self.original["id"])
            self.assertEqual(current["track_ids"], [self.tracks["fly"]["id"]])
            self.studio.feedback({"track_id": self.tracks["fly"]["id"], "verdict": "keep", "note": "New opinion entered during study"})
            self.studio.comparison_action({"comparison_id": current["id"], "preferred_track_id": self.tracks["fly"]["id"],
                                          "note": "Keep this concurrent session comment", "coherence": 4})
            return studied_score(source, seconds)
        with patch("flycomposer.organist_study.generate_study", side_effect=author), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render):
            batch = self.studio.study_organist(self.original["id"], FEEDBACK, seconds=45)
        self.assertEqual(batch["feedback"]["note"], "Keep this concurrent session comment")
        self.assertEqual(batch["feedback"]["preferred_track_id"], self.tracks["fly"]["id"])
        saved = Studio(self.root)
        self.assertEqual(saved.library["tracks"][0]["note"], "New opinion entered during study")
        self.assertEqual(saved.library["tracks"][1]["note"], "Retain this original opinion")
        self.assertEqual(saved.library["comparisons"][0], self.original_records["comparisons"][0])
        self.assertEqual(saved.library["tracks"][-1]["duration"], 45)
        self.assertEqual(saved.library["tracks"][0]["duration"], 75)
        self.assert_source_files_unchanged()

    def test_failed_author_keeps_only_the_reference_and_never_substitutes_music(self):
        with patch("flycomposer.organist_study.generate_study", side_effect=RuntimeError("Study model failed")) as model, \
             patch("flycomposer.organist.generate_performance", side_effect=AssertionError("Fallback author used")) as fallback, \
             patch("flycomposer.composition.write_song", side_effect=AssertionError("Template fallback used")), \
             patch("flycomposer.experiment.render_score", side_effect=AssertionError("Invented substitute rendered")) as render:
            batch = self.studio.study_organist(self.original["id"], FEEDBACK)
        model.assert_called_once(); fallback.assert_not_called(); render.assert_not_called()
        self.assertIn("Study model failed", batch["recording_errors"][0])
        self.assert_no_fake_organist(batch)
        # A generic recording retry must not silently discard the saved study
        # feedback and invoke the source-free authoring route.
        before = self.studio.index.read_bytes()
        with patch("flycomposer.organist.generate_performance") as fallback, \
             patch("flycomposer.organist_study.generate_study") as study:
            for operation in (self.studio.perform, self.studio.start_performance):
                with self.assertRaisesRegex(ValueError, "study"):
                    operation(retry_id=batch["id"])
                self.assertFalse(self.studio.busy)
        fallback.assert_not_called(); study.assert_not_called()
        self.assertEqual(self.studio.index.read_bytes(), before)

    def test_invalid_or_musically_unchanged_results_are_rejected_before_render(self):
        for mutation in ("empty", "performer", "seed", "bpm", "voices", "unchanged", "renumbered"):
            def author(root, source, feedback, seconds, progress):
                score = studied_score(source, seconds)
                if mutation in ("unchanged", "renumbered"):
                    score = copy.deepcopy(source)
                    score["title"] = "A new label is not new music"
                    if mutation == "renumbered":
                        score["events"].reverse()
                        for event in score["events"]:
                            event["id"] += 50
                elif mutation == "empty":
                    score["events"] = []
                elif mutation == "performer":
                    score["performer"] = "fly"
                elif mutation == "voices":
                    score["voices"]["lead"] = "classic:pluck"
                    for event in score["events"]:
                        event["voice"] = "classic:pluck"
                else:
                    score[mutation] += 1
                return rehash(score)
            with self.subTest(mutation=mutation), \
                 patch("flycomposer.organist_study.generate_study", side_effect=author), \
                 patch("flycomposer.experiment.render_score", side_effect=AssertionError("Invalid study was rendered")) as render:
                batch = self.studio.study_organist(self.original["id"], FEEDBACK)
            render.assert_not_called()
            self.assert_no_fake_organist(batch)

    def test_failed_index_publication_cannot_leave_a_dangling_organist_reference(self):
        def fail_publication(path, value):
            if Path(path) == self.studio.index and len(value.get("tracks", [])) > 2:
                raise OSError("Study index publication failed")
            return atomic_json(path, value)
        with patch("flycomposer.organist_study.generate_study", side_effect=lambda root, source, feedback, seconds, progress: studied_score(source, seconds)), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render), \
             patch("flycomposer.experiment.atomic_json", side_effect=fail_publication):
            batch = self.studio.study_organist(self.original["id"], FEEDBACK)
        self.assertIn("Study index publication failed", batch["recording_errors"][0])
        self.assert_no_fake_organist(batch)
        indexed = {track["id"] for track in Studio(self.root).library["tracks"]}
        self.assertTrue(all(set(c["track_ids"]) <= indexed for c in Studio(self.root).library["comparisons"]))

    def test_single_worker_reservation_and_failure_release(self):
        entered, release = threading.Event(), threading.Event()
        def author(root, source, feedback, seconds, progress):
            entered.set()
            if not release.wait(3):
                raise RuntimeError("Test worker was not released")
            raise ValueError("Expected asynchronous study failure")
        with patch("flycomposer.organist_study.generate_study", side_effect=author) as model, \
             patch("flycomposer.experiment.render_score", side_effect=AssertionError("Failed study rendered")):
            try:
                self.studio.start_organist_study(self.original["id"], FEEDBACK)
                self.assertTrue(entered.wait(2))
                self.assertTrue(self.studio.busy)
                for operation in (lambda: self.studio.start_organist_study(self.original["id"], FEEDBACK),
                                  lambda: self.studio.study_organist(self.original["id"], FEEDBACK),
                                  lambda: self.studio.start_performance(), lambda: self.studio.start_development(42)):
                    with self.assertRaisesRegex(ValueError, "already"):
                        operation()
            finally:
                release.set()
                deadline = time.monotonic() + 3
                while self.studio.busy and time.monotonic() < deadline:
                    time.sleep(.01)
        model.assert_called_once()
        self.assertFalse(self.studio.busy)
        self.assertIn("Expected asynchronous study failure", self.studio.error)
        self.assert_no_fake_organist(self.studio.library["comparisons"][-1])

    def test_invalid_request_or_missing_reference_refused_before_authoring(self):
        before = self.studio.index.read_bytes()
        with patch("flycomposer.organist_study.generate_study") as model:
            for source_id, feedback, seconds in (("unknown", FEEDBACK, 75), (None, FEEDBACK, 75),
                                                  (self.original["id"], "", 75), (self.original["id"], "   ", 75),
                                                  (self.original["id"], FEEDBACK, True), (self.original["id"], FEEDBACK, 121)):
                for operation in (self.studio.study_organist, self.studio.start_organist_study):
                    with self.subTest(source=source_id, seconds=seconds), self.assertRaises(ValueError):
                        operation(source_id, feedback, seconds=seconds)
                    self.assertFalse(self.studio.busy)
        model.assert_not_called()
        self.assertEqual(self.studio.index.read_bytes(), before)
        # Remove only the synthetic reference WAV within this test's temp root.
        (self.root / "library" / self.tracks["fly"]["id"] / "audio.wav").unlink()
        with patch("flycomposer.organist_study.generate_study") as model:
            with self.assertRaises(ValueError):
                self.studio.start_organist_study(self.original["id"], FEEDBACK)
        model.assert_not_called()
        self.assertEqual(self.studio.index.read_bytes(), before)
        self.assertFalse(self.studio.busy)


if __name__ == "__main__":
    unittest.main()

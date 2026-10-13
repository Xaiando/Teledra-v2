"""Independent-session integration, without model requests or production audio.

Authors and individual note synthesis are test doubles. The actual renderer,
WAV export, library transactions, worker reservation and HTTP route are exercised.
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
import urllib.error
import urllib.request
import wave

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from flycomposer.composition import ROLES
from flycomposer.experiment import Studio, atomic_json
from flycomposer.organist import expand_authoring
from flycomposer.render import LEVEL, PAN, assess_performance, render_score
from flycomposer.server import make_server


def authored_score(performer="organist", seconds=30, role="lead", quiet=False):
    proposal = {
        "title": "Explicit test capture", "bpm": 60,
        "voices": {role: "classic:keys"},
        "clips": [{"part": role, "events": [[60, 0, .25, .6], [64, .125, .25, .45]],
                   "placements": [[0, 0], [seconds - 3, -2]]}],
    }
    if quiet:
        proposal["voices"]["texture"] = "classic:bell"
        proposal["clips"].append({"part": "texture", "events": [[72, 0, .025, .001]],
                                  "placements": [[5, 0]]})
    score = expand_authoring(proposal, {"classic:keys", "classic:bell"}, seconds=seconds)
    score.update(performer=performer, condition=performer)
    return score


def fake_note(event, rate):
    """Distinct nonzero arrays make placement, overlap and omission observable."""
    frames = int(event["duration_seconds"] * rate)
    return (1 + (np.arange(frames) * 3 + event["id"]) % 11).astype(float)


def device_free_render(score, progress):
    return render_score(score, note_factory=fake_note, sr=800, progress=progress)


class PerformanceRenderTests(unittest.TestCase):
    def test_sparse_overlap_and_quiet_notes_pass_without_old_taste_gates(self):
        score = authored_score(quiet=True)
        original = copy.deepcopy(score)
        with patch("flycomposer.render.assess_score", side_effect=AssertionError("Legacy arrangement gate used")):
            audio, stems, audit = render_score(score, fake_note, 1000)
        self.assertEqual(score, original)
        self.assertEqual(audit["notes_mixed"], 5)
        self.assertEqual(audit["layers"], 2)
        self.assertEqual(audit["unique_lead_phrases"], 0)
        self.assertTrue(all(event["mixed"] for event in audit["events"]))
        self.assertGreater(np.max(np.abs(stems["texture"])), 0)
        self.assertLess(np.sqrt(np.mean(stems["texture"] ** 2)), 1e-5)
        self.assertTrue(np.all(audio[-1000:] == 0))
        self.assertIn("finite independent recording", audit["playback"])

    def test_exact_notes_independently_scattered_into_stems_and_stereo(self):
        score = authored_score(quiet=True)
        sr = 1000
        audio, stems, audit = render_score(score, fake_note, sr)
        expected = {role: np.zeros(int(score["duration_seconds"] * sr)) for role in ROLES}
        for event in score["events"]:
            raw = fake_note(event, sr)
            normalized = raw * (.08 * LEVEL[event["layer"]] * event["velocity"] /
                                np.sqrt(np.dot(raw, raw) / raw.size))
            positions = np.arange(raw.size) + int(event["start_seconds"] * sr)
            np.add.at(expected[event["layer"]], positions, normalized)
            ledger = next(item for item in audit["events"] if item["id"] == event["id"])
            self.assertEqual(ledger["start_frame"], int(event["start_seconds"] * sr))
            self.assertEqual(ledger["frames"], raw.size)
            self.assertEqual(ledger["note_pcm_sha256"], hashlib.sha256(normalized.astype("<f4").tobytes()).hexdigest())
        stereo = np.zeros_like(audio)
        for role in ROLES:
            np.testing.assert_allclose(stems[role], expected[role], rtol=0, atol=1e-14)
            angle = (PAN[role] + 1) * np.pi / 4
            stereo += expected[role][:, None] * np.array([np.cos(angle), np.sin(angle)])
        np.testing.assert_allclose(audio, stereo, rtol=0, atol=1e-14)

    def test_native_duration_corruption_and_incomplete_notes_fail_before_publication(self):
        native = authored_score(seconds=75)
        native["voices"]["lead"] = "piano"
        for event in native["events"]:
            event["voice"] = "piano"
        native["events"][0].update(duration_seconds=31, length_beats=31)
        native["event_sha256"] = hashlib.sha256(json.dumps(native["events"], sort_keys=True).encode()).hexdigest()
        with self.assertRaisesRegex(ValueError, "30 seconds"):
            assess_performance(native)
        corrupted = authored_score()
        corrupted["events"][0]["midi"] += 1
        with self.assertRaisesRegex(ValueError, "identity mismatch"):
            assess_performance(corrupted)
        factories = (
            lambda event, sr: np.zeros(int(event["duration_seconds"] * sr)),
            lambda event, sr: np.ones(int(event["duration_seconds"] * sr) - 1),
            lambda event, sr: np.full(int(event["duration_seconds"] * sr), np.nan),
        )
        for factory in factories:
            with self.subTest(factory=factory), self.assertRaises(ValueError):
                render_score(authored_score(), factory, 1000)


class PerformanceSessionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        (self.root / "data").mkdir()
        for name in ("circuit.npz", "circuit.json", "instruments.json"):
            shutil.copy2(ROOT / "data" / name, self.root / "data" / name)
        self.studio = Studio(self.root)
        self.old_id = "a" * 16
        self.old_folder = self.root / "library" / self.old_id
        self.old_folder.mkdir(parents=True)
        (self.old_folder / "audio.wav").write_bytes(b"Original recording: never overwrite")
        (self.old_folder / "score.json").write_text('{"original":true}', encoding="utf8")
        self.studio.library = {"version": 1, "tracks": [{
            "id": self.old_id, "title": "Saved original", "duration": 30, "condition": "connectome",
            "ensemble": "luminous", "verdict": "keep", "note": "An existing listener decision",
            "comparison_id": "original-session",
        }], "comparisons": [{"id": "original-session", "revealed": True,
                              "track_ids": [self.old_id], "feedback": {"note": "Retain this session"}}]}
        atomic_json(self.studio.index, self.studio.library)
        self.old_bytes = {path.name: path.read_bytes() for path in self.old_folder.iterdir()}

    def tearDown(self):
        self.temporary.cleanup()

    def assert_original_preserved(self):
        self.assertEqual({path.name: path.read_bytes() for path in self.old_folder.iterdir()}, self.old_bytes)
        saved = Studio(self.root)
        self.assertEqual(saved.library["tracks"][0]["note"], "An existing listener decision")
        self.assertEqual(saved.library["comparisons"][0]["feedback"], {"note": "Retain this session"})
        return saved

    def rate_published_fly(self):
        fly = next(track for track in self.studio.library["tracks"] if track.get("performer") == "fly")
        self.assertTrue((self.root / "library" / fly["id"] / "audio.wav").is_file())
        self.studio.feedback({"track_id": fly["id"], "verdict": "keep", "note": "Typed while Organist was preparing"})
        self.studio.comparison_action({"comparison_id": fly["comparison_id"],
                                       "preferred_track_id": fly["id"], "note": "A saved session comment", "motion": 4})
        return fly["id"]

    def test_first_author_survives_second_failure_and_concurrent_feedback(self):
        def organist(*args, **kwargs):
            self.rate_published_fly()
            raise RuntimeError("Actual author failed; do not substitute")
        with patch("flycomposer.autonomous.write_performance", return_value=authored_score("fly")) as fly_author, \
             patch("flycomposer.organist.generate_performance", side_effect=organist) as model_author, \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render), \
             patch("flycomposer.composition.write_song", side_effect=AssertionError("Fallback template used")), \
             patch("flycomposer.development.write_development", side_effect=AssertionError("Fallback development used")):
            batch = self.studio.perform(seconds=30)
        fly_author.assert_called_once()
        model_author.assert_called_once()
        self.assertEqual(batch["performer_status"], {"fly": "recorded", "organist": "failed"})
        self.assertEqual(len(batch["track_ids"]), 1)
        self.assertIn("Actual author failed", batch["recording_errors"][0])
        saved = self.assert_original_preserved()
        self.assertEqual(len(saved.library["tracks"]), 2)
        self.assertEqual(saved.library["tracks"][1]["verdict"], "keep")
        self.assertEqual(saved.library["tracks"][1]["note"], "Typed while Organist was preparing")
        self.assertEqual(saved.library["comparisons"][-1]["feedback"]["note"], "A saved session comment")
        self.assertFalse(self.studio.busy)

    def test_second_publication_preserves_prior_feedback_and_exports_real_files(self):
        def organist(*args, **kwargs):
            self.rate_published_fly()
            return authored_score("organist", role="answer")
        with patch("flycomposer.autonomous.write_performance", return_value=authored_score("fly")), \
             patch("flycomposer.organist.generate_performance", side_effect=organist), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render):
            batch = self.studio.perform(seconds=30)
        saved = self.assert_original_preserved()
        self.assertEqual(len(set(batch["track_ids"])), 2)
        self.assertEqual(batch["feedback"]["preferred_track_id"], batch["track_ids"][0])
        self.assertEqual(batch["feedback"]["motion"], 4)
        self.assertIsNone(saved.library["tracks"][-1]["verdict"])
        self.assertIsNone(self.studio.error)
        self.assertEqual(len(saved.state()["library"]), 3)
        for track_id in batch["track_ids"]:
            directory = self.root / "library" / track_id
            score = json.loads((directory / "score.json").read_text())
            audit = json.loads((directory / "audit.json").read_text())
            with wave.open(str(directory / "audio.wav")) as source:
                self.assertEqual((source.getframerate(), source.getnframes()), (800, int(score["duration_seconds"] * 800)))
                pcm = source.readframes(source.getnframes())
            self.assertEqual(hashlib.sha256(pcm).hexdigest(), audit["pcm_sha256"])
            self.assertEqual(audit["notes_mixed"], len(score["events"]))

    def test_mastering_protects_mono_stems_not_only_stereo_mix(self):
        with patch("flycomposer.autonomous.write_performance", return_value=authored_score("fly", role="bass")), \
             patch("flycomposer.organist.generate_performance", return_value=authored_score("organist", role="bass")), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render):
            batch = self.studio.perform(seconds=30)
        self.assertEqual(len(batch["track_ids"]), 2, batch["recording_errors"])
        for track_id in batch["track_ids"]:
            folder = self.root / "library" / track_id
            audit = json.loads((folder / "audit.json").read_text())
            source_peak = next(item["peak"] for item in audit["layer_stats"] if item["name"] == "bass")
            self.assertGreater(source_peak, audit["raw_peak"])
            self.assertAlmostEqual(audit["comparison_target_rms"], min(.055, .82 * audit["raw_rms"] / source_peak))
            with wave.open(str(folder / "bass.wav")) as source:
                mono = np.frombuffer(source.readframes(source.getnframes()), dtype="<i2") / 32767
            self.assertLessEqual(float(np.max(np.abs(mono))), .82002)

    def test_failed_second_index_transaction_never_publishes_dangling_reference(self):
        def fail_second_publication(path, value):
            performances = [track for track in value.get("tracks", []) if track.get("kind") == "performance"]
            if Path(path) == self.studio.index and len(performances) == 2:
                raise OSError("Injected second publication failure")
            return atomic_json(path, value)
        with patch("flycomposer.autonomous.write_performance", return_value=authored_score("fly")), \
             patch("flycomposer.organist.generate_performance", return_value=authored_score("organist")), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render), \
             patch("flycomposer.experiment.atomic_json", side_effect=fail_second_publication):
            batch = self.studio.perform(seconds=30)
        saved = self.assert_original_preserved()
        self.assertEqual(len(batch["track_ids"]), 1)
        self.assertEqual(batch["performer_status"]["organist"], "failed")
        self.assertIn("Injected second publication failure", batch["recording_errors"][0])
        indexed = {track["id"] for track in saved.library["tracks"]}
        for comparison in saved.library["comparisons"]:
            self.assertTrue(set(comparison["track_ids"]) <= indexed)
        self.assertEqual(len(indexed), 2)
        self.assertEqual(self.studio.library, saved.library)
        # Unindexed staged audio may remain for inspection; it must never become
        # a reference to an author reported as successfully published.
        self.assertEqual(len(saved.state()["library"]), 2)

    def partial_session(self):
        with patch("flycomposer.autonomous.write_performance", return_value=authored_score("fly")), \
             patch("flycomposer.organist.generate_performance", side_effect=ValueError("Original output limit")), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render):
            return self.studio.perform(seed=2026, seconds=30)

    def test_retry_only_missing_author_preserves_files_identity_and_latest_feedback(self):
        original = self.partial_session()
        fly_id = original["track_ids"][0]
        folder = self.root / "library" / fly_id
        before = {path.name: path.read_bytes() for path in folder.iterdir()}
        def organist(root, *, seed, seconds, progress):
            self.assertEqual((seed, seconds), (2026, 30))
            during = Studio(self.root).library["comparisons"][-1]
            self.assertEqual(during["performer_status"], {"fly": "recorded", "organist": "generating"})
            self.assertEqual(during["recording_errors"], [])
            self.assertEqual(during["retry_history"][0]["recording_errors"], original["recording_errors"])
            self.rate_published_fly()
            return authored_score("organist")
        with patch("flycomposer.autonomous.write_performance", side_effect=AssertionError("Successful author regenerated")) as fly, \
             patch("flycomposer.organist.generate_performance", side_effect=organist) as model, \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render) as render:
            # A retry uses the saved session parameters, not unrelated defaults.
            retried = self.studio.perform(seed=7, seconds=90, retry_id=original["id"])
        fly.assert_not_called()
        model.assert_called_once()
        render.assert_called_once()
        self.assertEqual(retried["id"], original["id"])
        self.assertEqual(retried["created_at"], original["created_at"])
        self.assertEqual(retried["track_ids"][0], fly_id)
        self.assertEqual(len(retried["track_ids"]), 2)
        self.assertEqual(retried["performer_status"], {"fly": "recorded", "organist": "recorded"})
        self.assertEqual(retried["recording_errors"], [])
        self.assertIsNone(self.studio.error)
        self.assertEqual({path.name: path.read_bytes() for path in folder.iterdir()}, before)
        saved = self.assert_original_preserved()
        self.assertEqual(len(saved.library["comparisons"]), 2)
        self.assertEqual(saved.library["tracks"][1]["note"], "Typed while Organist was preparing")
        self.assertEqual(saved.library["comparisons"][-1]["feedback"]["note"], "A saved session comment")
        self.assertIsNone(saved.library["tracks"][-1]["verdict"])

    def test_repeated_failed_retries_keep_prior_errors_and_never_regenerate_success(self):
        batch = self.partial_session()
        original_ids = list(batch["track_ids"])
        errors = [batch["recording_errors"]]
        with patch("flycomposer.autonomous.write_performance", side_effect=AssertionError("Successful author regenerated")) as fly, \
             patch("flycomposer.organist.generate_performance", side_effect=[ValueError("Retry one failed"), ValueError("Retry two failed")]) as model, \
             patch("flycomposer.experiment.render_score", side_effect=AssertionError("Failure substituted with audio")) as render:
            for attempt in (1, 2):
                batch = self.studio.perform(retry_id=batch["id"])
                self.assertEqual(batch["track_ids"], original_ids)
                self.assertEqual(len(batch["retry_history"]), attempt)
                self.assertEqual([entry["recording_errors"] for entry in batch["retry_history"]], errors)
                self.assertEqual(len(batch["recording_errors"]), 1)
                self.assertEqual(batch["performer_status"]["organist"], "failed")
                self.assertEqual(Studio(self.root).library["comparisons"][-1], batch)
                errors.append(batch["recording_errors"])
        fly.assert_not_called()
        render.assert_not_called()
        self.assertEqual(model.call_count, 2)
        self.assertEqual(len(self.studio.library["tracks"]), 2)
        self.assert_original_preserved()

    def test_retry_rejects_unknown_nonperformance_and_complete_sessions_before_work(self):
        with patch("flycomposer.autonomous.write_performance", return_value=authored_score("fly")), \
             patch("flycomposer.organist.generate_performance", return_value=authored_score("organist")), \
             patch("flycomposer.experiment.render_score", side_effect=device_free_render):
            complete = self.studio.perform(seconds=30)
        before = self.studio.index.read_bytes()
        with patch("flycomposer.autonomous.write_performance") as fly, \
             patch("flycomposer.organist.generate_performance") as model:
            for retry_id in ("unknown", "original-session", complete["id"], True, ""):
                for method in (self.studio.perform, self.studio.start_performance):
                    with self.subTest(retry_id=retry_id, method=method.__name__), self.assertRaises(ValueError):
                        method(retry_id=retry_id)
                    self.assertFalse(self.studio.busy)
        fly.assert_not_called()
        model.assert_not_called()
        self.assertEqual(self.studio.index.read_bytes(), before)

    def test_retry_supports_missing_fly_and_two_missing_authors(self):
        for first_succeeded in ("organist", None):
            with self.subTest(first_succeeded=first_succeeded):
                with patch("flycomposer.autonomous.write_performance", side_effect=ValueError("Fly failed")), \
                     patch("flycomposer.organist.generate_performance", **({"return_value": authored_score("organist")} if first_succeeded else {"side_effect": ValueError("Organist failed")})), \
                     patch("flycomposer.experiment.render_score", side_effect=device_free_render):
                    batch = self.studio.perform(seconds=30)
                previous_ids = list(batch["track_ids"])
                with patch("flycomposer.autonomous.write_performance", return_value=authored_score("fly")) as fly, \
                     patch("flycomposer.organist.generate_performance", return_value=authored_score("organist")) as model, \
                     patch("flycomposer.experiment.render_score", side_effect=device_free_render):
                    retry = self.studio.perform(retry_id=batch["id"])
                fly.assert_called_once()
                self.assertEqual(model.call_count, 0 if first_succeeded else 1)
                self.assertEqual(retry["track_ids"][:len(previous_ids)], previous_ids)
                self.assertEqual(len(retry["track_ids"]), 2)
                self.assertEqual(retry["recording_errors"], [])
                self.assertEqual(retry["id"], batch["id"])

    def test_http_retry_uses_saved_session_and_persists_generating_status(self):
        batch = self.partial_session()
        entered = threading.Event()
        release = threading.Event()
        def organist(root, *, seed, seconds, progress):
            self.assertEqual((seed, seconds), (2026, 30))
            entered.set()
            if not release.wait(4):
                raise ValueError("Test release timed out")
            return authored_score("organist")
        server = make_server(self.studio, 0)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with patch("flycomposer.autonomous.write_performance", side_effect=AssertionError("Successful author regenerated")) as fly, \
                 patch("flycomposer.organist.generate_performance", side_effect=organist), \
                 patch("flycomposer.experiment.render_score", side_effect=device_free_render):
                request = urllib.request.Request(f"http://127.0.0.1:{server.server_port}/api/perform",
                                                  data=json.dumps({"retry_id": batch["id"]}).encode(),
                                                  headers={"Content-Type": "application/json"})
                with opener.open(request, timeout=3) as response:
                    self.assertEqual(response.status, 202)
                self.assertTrue(entered.wait(2))
                self.assertTrue(self.studio.busy)
                self.assertEqual(Studio(self.root).library["comparisons"][-1]["performer_status"]["organist"], "generating")
                self.rate_published_fly()
                release.set()
                deadline = time.monotonic() + 3
                while self.studio.busy and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertFalse(self.studio.busy)
                self.assertIsNone(self.studio.error)
                fly.assert_not_called()
                saved = Studio(self.root).library["comparisons"][-1]
                self.assertEqual(saved["id"], batch["id"])
                self.assertEqual(len(saved["track_ids"]), 2)
                self.assertEqual(saved["feedback"]["note"], "A saved session comment")
        finally:
            release.set()
            server.shutdown()
            server.server_close()
            thread.join(3)

    def test_http_validation_and_one_worker_for_all_composition_modes(self):
        entered = threading.Event()
        release = threading.Event()
        def blocked_fly(*args, **kwargs):
            entered.set()
            if not release.wait(4):
                raise RuntimeError("Test worker release timed out")
            raise ValueError("Test Fly author failure")
        server = make_server(self.studio, 0)
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        def post(body):
            request = urllib.request.Request(f"http://127.0.0.1:{server.server_port}/api/perform",
                                              data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
            return opener.open(request, timeout=3)
        try:
            with patch("flycomposer.autonomous.write_performance", side_effect=blocked_fly), \
                 patch("flycomposer.organist.generate_performance", side_effect=ValueError("Test Organist failure")):
                for bad in ({"seed": True}, {"seed": -1}, {"seconds": True}, {"seconds": 29}, {"seconds": 75.5}, {"retry_id": "unknown"}, []):
                    with self.subTest(body=bad), self.assertRaises(urllib.error.HTTPError) as caught:
                        post(bad)
                    self.assertEqual(caught.exception.code, 400)
                    self.assertFalse(self.studio.busy)
                with post({"seed": 2026, "seconds": 30}) as response:
                    self.assertEqual(response.status, 202)
                self.assertTrue(entered.wait(2))
                with self.assertRaises(urllib.error.HTTPError) as caught:
                    post({"seed": 2027, "seconds": 30})
                self.assertEqual(caught.exception.code, 400)
                for operation in (lambda: self.studio.start_performance(),
                                  lambda: self.studio.start(42, "keys"),
                                  lambda: self.studio.start_development(42)):
                    with self.assertRaisesRegex(ValueError, "already"):
                        operation()
                release.set()
                deadline = time.monotonic() + 3
                while self.studio.busy and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertFalse(self.studio.busy)
                self.assertIn("Test Fly author failure", self.studio.error)
                self.assertIn("Test Organist failure", self.studio.error)
                self.assertEqual(len(self.studio.library["tracks"]), 1)
                self.assertEqual(self.studio.library["comparisons"][-1]["track_ids"], [])
        finally:
            release.set()
            server.shutdown()
            server.server_close()
            server_thread.join(3)


if __name__ == "__main__":
    unittest.main(verbosity=2)

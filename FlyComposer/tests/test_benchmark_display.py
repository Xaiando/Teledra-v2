"""The listening-room response masks benchmark identities without changing evidence."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from flycomposer.experiment import Studio


class BenchmarkDisplayTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.studio = Studio.__new__(Studio)
        self.studio.lock = threading.RLock()
        self.studio.index = Path(self.tmp.name) / "library/index.json"
        self.studio.busy = False
        self.studio.progress = "Ready"
        self.studio.error = None
        self.studio.catalog = []
        self.studio.circuit = {"metadata": {"nodes": 51, "edges": 1309,
            "region": "Test", "provenance_url": "https://example.test/", "node_kind": "types"}}
        tracks = [{"id": name, "title": f"{name} · authored title", "kind": "benchmark",
                   "performer": name, "condition": name, "description": f"Written by {name}",
                   "duration": 41, "verdict": None, "note": "", "voices": {},
                   "metrics": {"notes_scheduled": 30, "notes_mixed": 30, "layers": 5},
                   "benchmark_metrics": {"occupied_fraction": .8, "longest_gap_seconds": 1.2}}
                  for name in ("orator", "buzzart")]
        comparison = {"id": "round1", "title": "Composer benchmark · round 1", "kind": "benchmark",
                      "revealed": False, "track_ids": ["buzzart", "orator"], "feedback": {},
                      "benchmark": {"music_seconds": 40, "bpm": 96, "palette": "genesis",
                                    "performers": ["orator", "buzzart"]},
                      "variants": {t["id"]: {"display_name": t["performer"].title(),
                                               "description": t["description"]} for t in tracks}}
        self.studio.library = {"version": 1, "tracks": tracks, "comparisons": [comparison]}

    def test_blind_comparison_and_library_hide_display_identity_without_mutation(self):
        before = copy.deepcopy(self.studio.library)
        state = self.studio.state()
        comparison = state["comparisons"][0]
        self.assertEqual([t["display_name"] for t in comparison["tracks"]], ["Sample A", "Sample B"])
        self.assertEqual([t["condition"] for t in comparison["tracks"]], [None, None])
        self.assertNotIn("performers", comparison["benchmark"])
        self.assertEqual(comparison["variants"]["buzzart"]["display_name"], "Sample A")
        for track in state["library"]:
            self.assertEqual(track["performer"], "hidden")
            self.assertEqual(track["condition"], "hidden")
            self.assertNotIn("buzzart", track["title"].lower())
            self.assertNotIn("orator", track["description"].lower())
            self.assertEqual(track["metrics"]["notes_mixed"], 30)
            self.assertEqual(track["benchmark_metrics"]["longest_gap_seconds"], 1.2)
        self.assertEqual(self.studio.library, before)

    def test_reveal_preserves_feedback_and_restores_saved_author_labels(self):
        self.studio.comparison_action({"comparison_id": "round1", "preferred_track_id": "buzzart",
                                       "motion": 4, "note": "A has more motion"})
        self.studio.comparison_action({"comparison_id": "round1"}, reveal=True)
        state = self.studio.state()
        comparison = state["comparisons"][0]
        self.assertEqual(comparison["tracks"][0]["display_name"], "Buzzart")
        self.assertEqual(state["library"][0]["performer"], "buzzart")
        self.assertFalse(comparison["feedback"]["identities_revealed_at_rating"])
        self.assertEqual(comparison["feedback"]["note"], "A has more motion")
        saved = json.loads(self.studio.index.read_text(encoding="utf-8"))
        self.assertTrue(saved["comparisons"][0]["revealed"])
        self.assertEqual(saved["tracks"][0]["title"], "orator · authored title")

    def test_failed_round_retains_outcome_without_disclosing_surviving_author(self):
        comparison = self.studio.library["comparisons"][0]
        comparison["track_ids"] = ["buzzart"]
        comparison["recording_errors"] = ["Orator: model timed out"]
        comparison["performer_status"] = {"orator": "failed", "buzzart": "recorded"}
        blinded = self.studio.state()["comparisons"][0]
        self.assertEqual(len(blinded["tracks"]), 1)
        self.assertEqual(len(blinded["recording_errors"]), 1)
        self.assertNotIn("orator", blinded["recording_errors"][0].lower())
        self.assertNotIn("performer_status", blinded)
        self.studio.comparison_action({"comparison_id": "round1"}, reveal=True)
        revealed = self.studio.state()["comparisons"][0]
        self.assertEqual(revealed["recording_errors"], ["Orator: model timed out"])

    def test_existing_buzzart_named_sessions_remain_named(self):
        comparison = self.studio.library["comparisons"][0]
        comparison.update(kind="buzzart", revealed=True)
        for track in self.studio.library["tracks"]:
            track["kind"] = "buzzart"
        state = self.studio.state()
        self.assertEqual(state["comparisons"][0]["tracks"][0]["display_name"], "Buzzart")
        self.assertEqual(state["library"][0]["title"], "buzzart · authored title")


if __name__ == "__main__":
    unittest.main()

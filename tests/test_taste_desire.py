from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from taste_desire import apply_event, load_memory, prompt_context


class TasteDesireTests(unittest.TestCase):
    def test_immediate_desire_promotes_on_third_recurrence(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "taste.json"
            event = {"type": "desire", "want": "explore dungeon synth", "kind": "immediate", "strength": 0.5}
            apply_event(event, path, now=10)
            apply_event(event, path, now=20)
            apply_event(event, path, now=30)
            desire = load_memory(path)["desires"][0]
            self.assertEqual("persistent", desire["kind"])
            self.assertEqual(3, desire["recurrence"])

    def test_genre_like_survives_and_enters_prompt_context(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "taste.json"
            apply_event({"type": "like", "subject": "dungeon synth", "why": "atmospheric", "strength": 0.8}, path, now=10)
            apply_event({"type": "desire", "want": "build a lo-fi room", "kind": "persistent"}, path, now=11)
            context = prompt_context(path)
            self.assertIn("dungeon synth", context)
            self.assertIn("build a lo-fi room", context)


if __name__ == "__main__":
    unittest.main()

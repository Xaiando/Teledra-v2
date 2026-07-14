import os
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.dirname(ROOT))

import unittest
from kraken.kernel import game_profiles
from kraken.kernel import game_prompts

class TestGameProfiles(unittest.TestCase):
    def test_get_profile(self):
        p = game_profiles.get_profile("platformer")
        self.assertIsNotNone(p)
        self.assertEqual(p.name, "platformer")
        self.assertEqual(p.terminal_semantics, "finite")

        self.assertIsNone(game_profiles.get_profile("nonexistent"))

    def test_list_profiles(self):
        profiles = game_profiles.list_profiles()
        self.assertIn("platformer", profiles)
        self.assertIn("snake", profiles)
        self.assertIn("shooter", profiles)

    def test_resolve_trusted_profile_payload_only(self):
        payload = {"profile": "snake", "session": "finite", "contract_version": 2}
        res = game_profiles.resolve_trusted_profile(payload, None)
        self.assertEqual(res["profile"], "snake")
        self.assertEqual(res["session"], "finite")
        self.assertEqual(res["contract_version"], 2)
        self.assertEqual(res["source"], "payload")
        self.assertIsNone(res["error"])

    def test_resolve_trusted_profile_manifest_only(self):
        manifest = {"genre": "platformer", "session": "finite", "contract_version": 2}
        res = game_profiles.resolve_trusted_profile({}, manifest)
        self.assertEqual(res["profile"], "platformer")
        self.assertEqual(res["session"], "finite")
        self.assertEqual(res["contract_version"], 2)
        self.assertEqual(res["source"], "manifest")
        self.assertIsNone(res["error"])

    def test_resolve_trusted_profile_mismatch_fails(self):
        payload = {"profile": "snake", "session": "finite", "contract_version": 2}
        manifest = {"profile": "platformer", "session": "finite", "contract_version": 2}
        res = game_profiles.resolve_trusted_profile(payload, manifest)
        self.assertIsNotNone(res["error"])

    def test_resolve_trusted_profile_missing_fields_fails(self):
        payload = {"profile": "snake"}
        res = game_profiles.resolve_trusted_profile(payload, None)
        self.assertIsNotNone(res["error"])

    def test_genre_detection(self):
        self.assertEqual(game_prompts.detect_genre("side scrolling jumping adventure"), "platformer")
        self.assertEqual(game_prompts.detect_genre("collect food and grow tail in snake"), "snake")

    def test_verify_fixtures(self):
        import json
        from kraken.harness import verify_code
        
        manifest_path = os.path.join(ROOT, "tests", "fixtures", "games", "manifest.json")
        with open(manifest_path, "r", encoding="utf-8") as fh:
            manifest_data = json.load(fh)
            
        for fixture in manifest_data["fixtures"]:
            path_rel = os.path.join("tests", "fixtures", "games", fixture["path"])
            path_abs = os.path.join(ROOT, path_rel)
            
            # Setup mock job input
            job = {
                "input": json.dumps({
                    "profile": fixture["profile"],
                    "session": fixture["session"],
                    "contract_version": fixture["contract_version"],
                    "quality": "beast" if fixture["role"] == "positive" else ""
                })
            }
            
            # Write a temporary .kraken-game.json manifest inside the fixture dir to mock published target
            m_path = os.path.join(os.path.dirname(path_abs), ".kraken-game.json")
            with open(m_path, "w", encoding="utf-8") as mfh:
                json.dump({
                    "profile": fixture["profile"],
                    "session": fixture["session"],
                    "contract_version": fixture["contract_version"]
                }, mfh)
                
            try:
                result = {"ok": True, "output": path_rel}
                ctx = {"root": ROOT, "workdir": os.path.dirname(path_abs)}
                
                res = verify_code.verify(job, result, ctx)
                
                if fixture["role"] == "positive":
                    self.assertTrue(res["passed"], f"Positive fixture {fixture['path']} failed: {res['reasons']}")
                elif fixture["role"] == "negative":
                    self.assertFalse(res["passed"], f"Negative fixture {fixture['path']} should have failed")
                    expected_codes = fixture.get("expected_codes", [])
                    if expected_codes:
                        joined_reasons = " ".join(res["reasons"]).lower()
                        for code in expected_codes:
                            self.assertTrue(code.lower() in joined_reasons, f"Expected error code {code} in {res['reasons']}")
            finally:
                if os.path.exists(m_path):
                    os.remove(m_path)

if __name__ == "__main__":
    unittest.main()

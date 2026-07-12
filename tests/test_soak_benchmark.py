import json
import unittest
from unittest import mock

import soak_benchmark as soak


class SoakBenchmarkContractTests(unittest.TestCase):
    def test_dashboard_snapshot_parses_the_complete_pretty_json_document(self):
        payload = {
            "schema_version": 1,
            "health": {"overall": "observable"},
            "fractus": {"health": "ready"},
        }
        completed = {
            "ok": True,
            "duration_s": 0.1,
            "returncode": 0,
            "stdout_tail": json.dumps(payload, indent=2),
            "stderr_tail": "",
        }
        with mock.patch.object(soak, "run_cmd", return_value=completed):
            result = soak.snapshot_dashboard()

        self.assertEqual(result["parsed_health"], {"overall": "observable"})
        self.assertEqual(result["fractus_health"], "ready")

    def test_tts_dry_probe_never_turns_an_arbitrary_crash_into_success(self):
        crashed = {
            "ok": False,
            "duration_s": 0.1,
            "returncode": 2,
            "stdout_tail": "",
            "stderr_tail": "ImportError: broken backend",
        }
        with mock.patch.object(soak, "run_cmd", return_value=crashed):
            self.assertFalse(soak.light_tts_dry()["ok"])

        usage = {
            **crashed,
            "returncode": 1,
            "stderr_tail": "Usage: python generate_voice.py <text> <voice_name>",
        }
        with mock.patch.object(soak, "run_cmd", return_value=usage):
            self.assertTrue(soak.light_tts_dry()["ok"])


if __name__ == "__main__":
    unittest.main()

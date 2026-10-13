import copy
from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from flycomposer.organist import (MAX_CLIP_NOTES, MAX_CLIPS, MAX_EVENTS, MAX_OUTPUT_TOKENS,
                                 MAX_PLACEMENTS, OrganistError, _json, expand_authoring,
                                 generate_performance)


def proposal():
    return {"title": "Model-owned recording", "bpm": 60,
            "voices": {"lead": "classic:keys"},
            "clips": [{"part": "lead", "events": [[60, 0, 4, .6], [64, 1.5, 2, .45]],
                       "placements": [[0, 0], [23, -2], [46, 5], [69, 0]]}]}


@contextmanager
def fake_ollama(replies):
    requests = []
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass
        def do_POST(self):
            requests.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
            index = len(requests) - 1
            self.send_response(200)
            self.send_header("Content-Type", "application/x-ndjson")
            self.end_headers()
            if index >= len(replies):
                return
            reply = replies[index]
            packets = reply if isinstance(reply, list) else [
                {"model": "qwen2.5:7b", "message": {"role": "assistant", "content": reply[:len(reply)//2]}, "done": False},
                {"model": "qwen2.5:7b", "message": {"role": "assistant", "content": reply[len(reply)//2:]}, "done": False},
                {"model": "qwen2.5:7b", "message": {"role": "assistant", "content": ""}, "done": True,
                 "done_reason": "stop", "eval_count": 123, "prompt_eval_count": 1000, "total_duration": 1000000000},
            ]
            for packet in packets:
                self.wfile.write((json.dumps(packet) + "\n").encode())
                self.wfile.flush()
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with patch.dict("os.environ", {"TELEDRA_OLLAMA_URL": f"http://127.0.0.1:{server.server_port}", "TELEDRA_MODEL": "qwen2.5:7b"}):
            yield requests
    finally:
        server.shutdown()
        server.server_close()
        worker.join(3)


class AuthoringTests(unittest.TestCase):
    def test_exact_events_overlaps_placements_and_transposes(self):
        authored = proposal()
        original = copy.deepcopy(authored)
        score = expand_authoring(authored, {"classic:keys"})
        self.assertEqual(authored, original)
        self.assertEqual(score["authoring"], original)
        self.assertEqual(score["performer"], "organist")
        self.assertEqual(score["kind"], "performance")
        self.assertEqual(score["duration_seconds"], 74)
        self.assertEqual(score["capture_limit_seconds"], 75)
        self.assertEqual(len(score["events"]), 8)
        self.assertEqual(set(score["voices"]), {"lead"})
        for event in score["events"]:
            clip = authored["clips"][event["source_clip"]]
            start, transpose = clip["placements"][event["source_placement"]]
            midi, offset, length, velocity = clip["events"][event["source_event"]]
            self.assertEqual((event["midi"], event["start_seconds"], event["duration_seconds"], event["velocity"]),
                             (midi + transpose, start + offset, length, velocity))
        self.assertEqual(score["event_sha256"], hashlib.sha256(json.dumps(score["events"], sort_keys=True).encode()).hexdigest())

    def test_sparse_single_voice_is_not_a_music_quality_failure(self):
        authored = {"title": "Held recording", "bpm": 60, "voices": {"texture": "classic:pad"},
                    "clips": [{"part": "texture", "events": [[60, 0, 74, .3]], "placements": [[0, 0]]}]}
        score = expand_authoring(authored, {"classic:pad"})
        self.assertEqual(len(score["events"]), 1)
        self.assertEqual(score["release_seconds"], 1)

    def test_rejects_invalid_event_values_and_never_clamps(self):
        mutations = [
            lambda p: p.update(bpm=True),
            lambda p: p.update(bpm=float("nan")),
            lambda p: p["clips"][0]["events"][0].__setitem__(0, 60.0),
            lambda p: p["clips"][0]["events"][0].__setitem__(1, -1),
            lambda p: p["clips"][0]["events"][0].__setitem__(2, 0),
            lambda p: p["clips"][0]["events"][0].__setitem__(3, 1.01),
            lambda p: p["clips"][0]["events"][0].__setitem__(3, 0),
            lambda p: p["clips"][0]["placements"][0].__setitem__(1, 100),
            lambda p: p["clips"][0]["placements"][-1].__setitem__(0, 74),
            lambda p: p["voices"].update(lead="unknown"),
            lambda p: p["voices"].update(bass="classic:keys"),
            lambda p: p["clips"][0].update(part="unknown"),
            lambda p: p.update(python="print('not executed')"),
        ]
        for mutate in mutations:
            authored = proposal()
            mutate(authored)
            with self.subTest(authoring=repr(authored)):
                with self.assertRaises(ValueError):
                    expand_authoring(authored, {"classic:keys"})

    def test_native_note_limit_and_start_boundary(self):
        authored = proposal()
        authored["voices"]["lead"] = "piano"
        authored["clips"][0]["events"][0][2] = 31
        with self.assertRaisesRegex(ValueError, "30-second"):
            expand_authoring(authored, {"piano"})
        authored = proposal()
        authored["clips"][0]["placements"] = [[6, 0]]
        with self.assertRaisesRegex(ValueError, "begin by 5s"):
            expand_authoring(authored, {"classic:keys"})

    def test_author_may_end_early_with_exact_notes_and_one_second_tail(self):
        authored = {"title": "An authored ending", "bpm": 60, "voices": {"lead": "piano"},
                    "clips": [{"part": "lead", "events": [[62, 0, 15, .4]], "placements": [[0, 0]]}]}
        original = copy.deepcopy(authored)
        score = expand_authoring(authored, {"piano"}, seconds=75)
        self.assertEqual(score["authoring"], original)
        self.assertEqual(score["duration_seconds"], 16)
        self.assertEqual(score["musical_end_seconds"], 15)
        self.assertEqual(score["release_seconds"], 1)
        self.assertEqual(score["sections"][0]["end_seconds"], 16)
        self.assertEqual(len(score["events"]), 1)
        event = score["events"][0]
        self.assertEqual((event["midi"], event["start_seconds"], event["duration_seconds"], event["velocity"]), (62, 0, 15, .4))
        authored["clips"][0]["events"][0][2] = 3
        short = expand_authoring(authored, {"piano"}, seconds=75)
        self.assertEqual(short["duration_seconds"], 10)
        self.assertEqual(short["events"][0]["duration_seconds"], 3)

    def test_compact_authoring_limits_are_enforced_without_silent_truncation(self):
        for field, limit in (("events", MAX_CLIP_NOTES), ("placements", MAX_PLACEMENTS)):
            authored = proposal()
            authored["clips"][0][field] = [authored["clips"][0][field][0]] * (limit + 1)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, f"1..{limit} entries"):
                expand_authoring(authored, {"classic:keys"})
        authored = proposal()
        authored["clips"] *= MAX_CLIPS + 1
        with self.assertRaisesRegex(ValueError, f"1..{MAX_CLIPS} entries"):
            expand_authoring(authored, {"classic:keys"})

    def test_maximum_compact_plan_has_bounded_expansion(self):
        authored = proposal()
        clip = {"part": "lead", "events": [[60, 0, 4, .5]] * MAX_CLIP_NOTES,
                "placements": [[69 * i / (MAX_PLACEMENTS - 1), 0] for i in range(MAX_PLACEMENTS)]}
        authored["clips"] = [copy.deepcopy(clip) for _ in range(MAX_CLIPS)]
        score = expand_authoring(authored, {"classic:keys"})
        self.assertEqual(len(score["events"]), MAX_CLIPS * MAX_CLIP_NOTES * MAX_PLACEMENTS)
        self.assertLessEqual(len(score["events"]), MAX_EVENTS)

    def test_nonfinite_and_duplicate_json_keys_rejected(self):
        for text in ('{"bpm":NaN}', '{"bpm":Infinity}', '{"bpm":60,"bpm":70}'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                _json(text)


class OllamaAdapterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "data").mkdir()
        (self.root / "profiles").mkdir()
        shutil.copy2(ROOT / "data/instruments.json", self.root / "data/instruments.json")
        self.profile = b"You are the authored Organist persona.\n"
        (self.root / "profiles/organist.md").write_bytes(self.profile)

    def tearDown(self):
        self.tmp.cleanup()

    def test_real_http_contract_records_profile_catalog_and_complete_stream(self):
        progress = []
        raw = json.dumps(proposal())
        with fake_ollama([raw]) as requests:
            score = generate_performance(self.root, progress=progress.append)
        self.assertEqual(len(requests), 1)
        self.assertEqual(requests[0]["model"], "qwen2.5:7b")
        self.assertTrue(requests[0]["stream"])
        self.assertEqual(requests[0]["options"]["seed"], 2026)
        self.assertEqual(requests[0]["options"]["num_predict"], MAX_OUTPUT_TOKENS)
        clip_schema = requests[0]["format"]["properties"]["clips"]
        self.assertEqual(clip_schema["maxItems"], MAX_CLIPS)
        self.assertEqual(clip_schema["items"]["properties"]["events"]["maxItems"], MAX_CLIP_NOTES)
        self.assertEqual(clip_schema["items"]["properties"]["placements"]["maxItems"], MAX_PLACEMENTS)
        prompt = requests[0]["messages"][0]["content"]
        self.assertTrue(prompt.startswith(self.profile.decode()))
        catalog = json.loads((self.root / "data/instruments.json").read_text())
        for voice in catalog:
            self.assertIn('"id":"' + voice["id"] + '"', prompt)
        self.assertIn("classic:keys", prompt)
        self.assertIn("Use compact JSON", prompt)
        self.assertIn("do not flatten copies into hundreds of note rows", prompt)
        self.assertIn("UP TO 75 seconds", prompt)
        self.assertIn("an earlier ending is accepted", prompt)
        provenance = score["inference_provenance"]
        self.assertEqual(provenance["profile_sha256"], hashlib.sha256(self.profile).hexdigest())
        self.assertIn("does not share live Court", provenance["scope"])
        self.assertEqual(provenance["capture_limit_seconds"], 75)
        self.assertEqual(provenance["recorded_seconds"], score["duration_seconds"])
        directory = Path(provenance["output_dir"])
        self.assertEqual((directory / "source-profile.md").read_bytes(), self.profile)
        self.assertEqual((directory / "authoring-0.txt").read_text(), raw)
        self.assertEqual(len((directory / "response-0.ndjson").read_text().splitlines()), 3)
        self.assertEqual(json.loads((directory / "score.json").read_text())["events"], score["events"])
        self.assertTrue(any("8 explicit notes" in item for item in progress))

    def test_one_factual_repair_preserves_model_authored_correction(self):
        bad = proposal()
        bad["clips"][0]["placements"][-1][0] = 80
        good = proposal()
        good["title"] = "The model corrected its own recording"
        with fake_ollama([json.dumps(bad), json.dumps(good)]) as requests:
            score = generate_performance(self.root)
        self.assertEqual(len(requests), 2)
        self.assertEqual(requests[1]["messages"][-2]["content"], json.dumps(bad))
        repair = requests[1]["messages"][-1]["content"]
        self.assertIn("Factual validation error", repair)
        self.assertEqual(score["authoring"], good)
        self.assertEqual([a["status"] for a in score["inference_provenance"]["attempts"]], ["invalid_authoring", "accepted_authoring"])

    def test_two_bad_outputs_fail_without_fallback_and_retain_evidence(self):
        with fake_ollama(['{"not":"a score"}', '{"still":"invalid"}']) as requests:
            with self.assertRaises(OrganistError) as caught:
                generate_performance(self.root)
        self.assertEqual(len(requests), 2)
        self.assertIn("No substitute piece", str(caught.exception))
        directory = caught.exception.output_dir
        self.assertTrue((directory / "request-1.json").exists())
        self.assertFalse((directory / "score.json").exists())

    def test_truncated_stream_is_not_accepted_even_if_json_is_parseable(self):
        packets = [{"message": {"content": json.dumps(proposal())}, "done": False}]
        with fake_ollama([packets]) as requests:
            with self.assertRaisesRegex(OrganistError, "before completion"):
                generate_performance(self.root)
        self.assertEqual(len(requests), 1)

    def test_nonlocal_endpoint_rejected_before_sending_profile(self):
        with patch.dict("os.environ", {"TELEDRA_OLLAMA_URL": "https://example.com"}):
            with self.assertRaisesRegex(ValueError, "loopback"):
                generate_performance(self.root)
        self.assertFalse((self.root / "reports").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)

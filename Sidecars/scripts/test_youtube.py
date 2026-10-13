"""Regression coverage for YouTube URL ingest and honest caption fallbacks."""

import io
import json
import sys
import time
import types
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import youtube


VIDEO_ID = "jNQXAC9IVRw"
URL = "https://www.youtube.com/watch?v=" + VIDEO_ID


def track(code, generated=False):
    return types.SimpleNamespace(language_code=code, language=code, is_generated=generated)


class YouTubeUrlTests(unittest.TestCase):
    def test_shared_url_variants(self):
        for url in (
            URL,
            URL + "&t=32&si=shared",
            "youtube.com/watch?v=" + VIDEO_ID,
            "https://m.youtube.com/watch?v=" + VIDEO_ID,
            "https://music.youtube.com/watch?feature=share&amp;v=" + VIDEO_ID,
            "https://youtu.be/" + VIDEO_ID + "?si=shared",
            "youtu.be/" + VIDEO_ID,
            "//www.youtube.com/watch?v=" + VIDEO_ID,
            "https://www.youtube.com/shorts/" + VIDEO_ID,
            "https://www.youtube.com/live/" + VIDEO_ID,
            "https://www.youtube-nocookie.com/embed/" + VIDEO_ID,
            "https://www.youtube.com/attribution_link?u=%2Fwatch%3Fv%3D" + VIDEO_ID,
            "<" + URL + ">",
            URL + ").",
        ):
            with self.subTest(url=url):
                self.assertEqual(youtube.video_id(url), VIDEO_ID)

    def test_rejects_lookalikes_non_video_links_and_bad_ids(self):
        for url in (
            "https://notyoutube.com/watch?v=" + VIDEO_ID,
            "https://youtube.com.evil.test/watch?v=" + VIDEO_ID,
            "https://youtube.com@evil.test/watch?v=" + VIDEO_ID,
            "https://user@youtube.com/watch?v=" + VIDEO_ID,
            "ftp://youtube.com/watch?v=" + VIDEO_ID,
            "https://youtube.com/playlist?list=" + VIDEO_ID,
            "https://youtube.com/watch?v=short",
            "https://youtube.com/watch?v=" + VIDEO_ID + "extra",
            "https://youtube.com/attribution_link?u=https%3A%2F%2Fevil.test",
            "https://[invalid/watch?v=" + VIDEO_ID,
            "", None,
        ):
            with self.subTest(url=url):
                self.assertEqual(youtube.video_id(url), "")


class TranscriptTests(unittest.TestCase):
    def test_prefers_english_and_manual_captions(self):
        norwegian, english_auto, english_manual = track("no"), track("en", True), track("en-US")
        self.assertIs(youtube.choose_transcript([norwegian, english_auto, english_manual], ["en"]), english_manual)

    def test_non_english_video_still_yields_evidence(self):
        norwegian = track("no", True)
        self.assertIs(youtube.choose_transcript([norwegian], ["en"]), norwegian)

    def test_requested_language_has_priority(self):
        german, english = track("de", True), track("en")
        self.assertIs(youtube.choose_transcript([english, german], ["de", "en"]), german)

    def test_modern_api_gets_a_bounded_http_session(self):
        selected = track("no")
        selected.fetch = lambda: [types.SimpleNamespace(text="Hei!")]

        class FakeApi:
            def __init__(self, http_client):
                self.http_client = http_client

            def list(self, identifier):
                self.http_client.get("https://www.youtube.com/watch?v=" + identifier)
                return [selected]

        with patch.dict(sys.modules, {"youtube_transcript_api": types.SimpleNamespace(YouTubeTranscriptApi=FakeApi)}):
            with patch("requests.Session.request") as request:
                snippets, metadata = youtube.fetch_transcript(VIDEO_ID, ["en"], time.monotonic() + 20)
                self.assertEqual(snippets[0].text, "Hei!")
                self.assertEqual(metadata["language_code"], "no")
                self.assertGreater(request.call_args.kwargs["timeout"], 0)
                self.assertLessEqual(request.call_args.kwargs["timeout"], 6)

    def test_legacy_api_dict_snippets_still_work(self):
        selected = track("en")
        selected.fetch = lambda: [{"text": "Legacy captions"}]
        api = types.SimpleNamespace(list_transcripts=lambda identifier: [selected])
        with patch.dict(sys.modules, {"youtube_transcript_api": types.SimpleNamespace(YouTubeTranscriptApi=api)}):
            snippets, _ = youtube.fetch_transcript(VIDEO_ID, ["en"], time.monotonic() + 20)
            self.assertEqual(youtube.transcript_excerpt(snippets)[0], "Legacy captions")

    def test_short_transcript_preserves_unicode_and_decodes_entities(self):
        excerpt, total, truncated = youtube.transcript_excerpt([
            {"text": "  Hello\nworld &amp; "}, types.SimpleNamespace(text="こんにちは")
        ])
        self.assertEqual(excerpt, "Hello world & こんにちは")
        self.assertEqual(total, len(excerpt))
        self.assertFalse(truncated)

    def test_long_recording_includes_the_end_with_explicit_gaps(self):
        text = "INTRO " + "early middle late " * 4_000 + "CONCLUSION"
        excerpt, total, truncated = youtube.transcript_excerpt([{"text": text}])
        self.assertTrue(truncated)
        self.assertEqual(total, len(text))
        self.assertLessEqual(len(excerpt), youtube.MAX_TRANSCRIPT_CHARS)
        self.assertTrue(excerpt.startswith("INTRO"))
        self.assertTrue(excerpt.endswith("CONCLUSION"))
        self.assertEqual(excerpt.count("[... transcript excerpt gap ...]"), 3)


class ResponseTests(unittest.TestCase):
    def test_transcript_succeeds_without_metadata(self):
        with patch.object(youtube, "fetch_metadata", return_value={}):
            with patch.object(youtube, "fetch_transcript", return_value=([{"text": "Actual caption evidence"}], {})):
                response = youtube.handle_request({"url": "youtu.be/" + VIDEO_ID})
        self.assertTrue(response["ok"])
        self.assertEqual(response["data"]["content_status"], "transcript")
        self.assertEqual(response["data"]["url"], URL)
        self.assertEqual(response["data"]["transcript"], "Actual caption evidence")

    def test_unavailable_captions_return_clearly_limited_metadata(self):
        with patch.object(youtube, "fetch_metadata", return_value={"title": "Video title", "author": "Channel"}):
            with patch.object(youtube, "fetch_transcript", side_effect=RuntimeError("No captions")):
                response = youtube.handle_request({"url": URL})
        self.assertTrue(response["ok"])
        self.assertEqual(response["data"]["content_status"], "metadata_only")
        self.assertEqual(response["data"]["transcript"], "")
        self.assertEqual(response["data"]["transcript_error"], "No captions")
        self.assertIn("Do not claim", response["data"]["evidence_limit"])

    def test_missing_captions_and_metadata_return_failure(self):
        with patch.object(youtube, "fetch_metadata", return_value={}):
            with patch.object(youtube, "fetch_transcript", side_effect=TimeoutError()):
                response = youtube.handle_request({"url": URL})
        self.assertFalse(response["ok"])
        self.assertIn("timed out", response["message"])

    def test_import_error_gives_configured_python_repair_hint(self):
        self.assertIn("TELEDRA_PYTHON", youtube.failure_message(ModuleNotFoundError("youtube_transcript_api")))

    def test_invalid_input_never_fetches_network(self):
        with patch.object(youtube, "fetch_metadata") as fetch:
            with self.assertRaisesRegex(ValueError, "valid youtube.com"):
                youtube.handle_request({"url": "https://notyoutube.com/watch?v=" + VIDEO_ID})
            fetch.assert_not_called()

    def test_unicode_output_survives_windows_ascii_pipe(self):
        buffer = io.BytesIO()
        output = io.TextIOWrapper(buffer, encoding="ascii")
        response = {"ok": True, "capability": "youtube", "message": "日本語", "data": {"transcript": "こんにちは 🌏"}}
        with patch.object(youtube, "handle_request", return_value=response):
            with patch.object(sys, "stdin", io.StringIO("{}")), patch.object(sys, "stdout", output):
                self.assertEqual(youtube.main(), 0)
        output.flush()
        self.assertEqual(json.loads(buffer.getvalue()), response)

    def test_malformed_json_returns_one_protocol_response(self):
        output = io.StringIO()
        with patch.object(sys, "stdin", io.StringIO("not json")), patch.object(sys, "stdout", output):
            self.assertEqual(youtube.main(), 2)
        self.assertFalse(json.loads(output.getvalue())["ok"])


if __name__ == "__main__":
    unittest.main()

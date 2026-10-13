"""Regression checks for the actual worker, without loading model weights."""
import hashlib
import io
import os
from pathlib import Path
import re
import struct
import unittest
from unittest.mock import Mock, patch

import numpy as np

import voice_worker as worker


class WorkerTests(unittest.TestCase):
    def voice(self):
        voice = worker.Voice.__new__(worker.Voice)
        voice._base = os.path.dirname(worker.__file__)
        voice._prompts = {}
        voice._engine = Mock()
        voice._engine.encode_prompt.return_value = {"encoded": True}
        voice._engine.generate_speech.return_value = np.ones(480, dtype=np.float32) * 0.1
        return voice

    def test_every_rust_voice_has_its_own_complete_worker_reference(self):
        root = Path(worker.__file__).resolve().parents[2]
        rust = (root / "TTS/src/voice.rs").read_text(encoding="utf-8")
        slugs = re.findall(r'VoiceName::\w+ => "([a-z]+)"', rust)
        self.assertEqual(set(worker.REFERENCES), set(slugs))
        self.assertEqual(len(slugs), len(worker.REFERENCES))
        paths = set()
        with patch.dict(os.environ, {"TELEDRA_QUEEN_VOICE": "clean"}):
            voice = self.voice()
            for slug, relative in worker.REFERENCES.items():
                with self.subTest(voice=slug):
                    path = Path(worker.reference_for(slug, voice._base)).resolve()
                    self.assertEqual(path, (Path(voice._base) / relative).resolve())
                    self.assertNotIn(path, paths, "each speaker needs their own recording")
                    paths.add(path)
                    duration = worker.reference_duration(str(path))
                    voice.prompt_for(slug)
                    self.assertEqual(Path(voice._engine.encode_prompt.call_args.args[0]), path)
                    self.assertGreaterEqual(voice._engine.encode_prompt.call_args.kwargs["duration"], duration)
                    calls = voice._engine.encode_prompt.call_count
                    voice.prompt_for(slug)
                    self.assertEqual(voice._engine.encode_prompt.call_count, calls)
        self.assertEqual(len(voice._prompts), len(worker.REFERENCES))

    def test_court_recordings_do_not_reuse_teledras_audio(self):
        base = Path(worker.__file__).resolve().parent
        digest = lambda relative: hashlib.sha256((base / relative).read_bytes()).digest()
        queen_audio = {digest(relative) for relative in worker.QUEEN_REFERENCES.values()}
        recordings = {slug: digest(relative) for slug, relative in worker.REFERENCES.items()}
        for slug, audio in recordings.items():
            if slug != "queen":
                with self.subTest(voice=slug):
                    self.assertNotIn(audio, queen_audio, "court member reuses Teledra's audio")
        # Every office speaks with its own recording. The Treasurer once shared
        # the Archivist's file byte for byte, so the two sounded identical.
        for slug, audio in recordings.items():
            with self.subTest(own_voice=slug):
                self.assertNotIn(
                    audio,
                    {audio for other, audio in recordings.items() if other != slug},
                    "court member shares another member's recording",
                )

    def assert_failed_voice_preserves_next_request(self, voice, slug, error):
        stream = io.BytesIO()
        with self.assertRaisesRegex(error, re.escape(slug)):
            voice.speak(slug, "This speaker must keep their own voice.", worker.AudioOut(stream))
        self.assertEqual(stream.getvalue(), struct.pack("<ii", 48000, 0))
        voice._engine.encode_prompt.assert_not_called()
        voice._engine.generate_speech.assert_not_called()
        self.assertEqual(voice._prompts, {})
        # The reference failure ends exactly one request. A valid speaker can
        # still synthesize immediately afterward without restarting the worker.
        next_slug = "orator" if slug == "organist" else "organist"
        voice.speak(next_slug, "The next complete sentence follows.", worker.AudioOut(stream))
        stream.seek(8)
        self.assertEqual(struct.unpack("<ii", stream.read(8)), (48000, 480))
        stream.read(480 * 4)
        self.assertEqual(stream.read(), struct.pack("<i", 0))

    def test_unknown_voice_is_rejected_without_substituting_the_queen(self):
        self.assert_failed_voice_preserves_next_request(self.voice(), "jester", ValueError)

    def test_missing_reference_is_rejected_without_substituting_the_queen(self):
        with patch.dict(os.environ, {"TELEDRA_QUEEN_VOICE": "custom"}):
            for slug in worker.REFERENCES:
                with self.subTest(voice=slug):
                    references = worker.QUEEN_REFERENCES if slug == "queen" else worker.REFERENCES
                    key = "custom" if slug == "queen" else slug
                    with patch.dict(references, {key: "missing-test-recording.wav"}):
                        self.assert_failed_voice_preserves_next_request(
                            self.voice(), slug, FileNotFoundError
                        )

    def test_every_mode_uses_the_selected_complete_reference(self):
        for mode, relative in worker.QUEEN_REFERENCES.items():
            with self.subTest(mode=mode), patch.dict(os.environ, {"TELEDRA_QUEEN_VOICE": mode}):
                voice = self.voice()
                voice.prompt_for("queen")
                path, = voice._engine.encode_prompt.call_args.args
                duration = voice._engine.encode_prompt.call_args.kwargs["duration"]
                self.assertEqual(os.path.basename(path), os.path.basename(relative))
                self.assertGreaterEqual(duration, worker.reference_duration(path))
                voice.prompt_for("queen")
                self.assertEqual(voice._engine.encode_prompt.call_count, 1)

    def test_clean_reference_is_not_cropped_inside_magnificently(self):
        with patch.dict(os.environ, {"TELEDRA_QUEEN_VOICE": "clean"}):
            voice = self.voice()
            voice.prompt_for("queen")
            self.assertGreater(voice._engine.encode_prompt.call_args.kwargs["duration"], 6.8)

    def test_speed_is_finite_bounded_and_used(self):
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(worker.speech_speed(), .90)
        for raw, expected in [("nan", .90), ("inf", .90), ("bad", .90), ("0", .55), ("3", 1.1), ("0.8", .8)]:
            with patch.dict(os.environ, {"TELEDRA_TTS_SPEED": raw}):
                self.assertEqual(worker.speech_speed(), expected)
        voice = self.voice()
        with patch.dict(os.environ, {"TELEDRA_TTS_SPEED": "0.8"}):
            voice.speak("queen", "**The music has room to breathe.**", worker.AudioOut(io.BytesIO()))
        self.assertEqual(voice._engine.generate_speech.call_args.kwargs["speed"], .8)
        self.assertTrue(voice._engine.generate_speech.call_args.kwargs["return_smooth"])
        self.assertNotIn("*", voice._engine.generate_speech.call_args.args[0])

    def test_warmup_and_oom_retry_use_the_same_smooth_vocoder(self):
        voice = self.voice()
        voice.warm_up("queen")
        self.assertTrue(voice._engine.generate_speech.call_args.kwargs["return_smooth"])
        voice._engine.generate_speech.reset_mock()
        voice._engine.generate_speech.side_effect = [
            RuntimeError("CUDA out of memory"), np.ones((1, 480), dtype=np.float32) * .1
        ]
        with patch("torch.cuda.is_available", return_value=True), patch("torch.cuda.empty_cache"):
            voice.speak("queen", "The next complete sentence follows.", worker.AudioOut(io.BytesIO()))
        self.assertEqual(voice._engine.generate_speech.call_count, 2)
        for call in voice._engine.generate_speech.call_args_list:
            self.assertTrue(call.kwargs["return_smooth"])

    def test_sentence_edges_join_silence_without_clicks_or_lost_time(self):
        voice = self.voice()
        raw = np.full((1, 4_800), .1, dtype=np.float32)
        voice._engine.generate_speech.return_value = raw
        stream = io.BytesIO()
        voice.speak("queen", "The first sentence needs a quiet ending. The next sentence starts gently too.", worker.AudioOut(stream))
        stream.seek(0)
        self.assertEqual(struct.unpack("<i", stream.read(4))[0], 48_000)
        frames = []
        while count := struct.unpack("<i", stream.read(4))[0]:
            frames.append(np.frombuffer(stream.read(count * 4), dtype="<f4"))
        self.assertEqual([len(frame) for frame in frames], [4_800, 4_800, 4_800])
        for frame in [frames[0], frames[2]]:
            self.assertEqual(frame[0], 0.0)
            self.assertEqual(frame[-1], 0.0)
            np.testing.assert_array_equal(frame[240:-240], raw[0, 240:-240])
        np.testing.assert_array_equal(raw, np.full((1, 4_800), .1, dtype=np.float32))
        self.assertTrue(np.all(frames[1] == 0))
        self.assertLess(float(np.max(np.abs(np.diff(np.concatenate(frames))))), .001)

    def test_bad_audio_shape_or_nonfinite_samples_never_reach_playback(self):
        for raw in [np.zeros((2, 480)), np.array([np.nan]), np.array([np.inf])]:
            voice = self.voice()
            voice._engine.generate_speech.return_value = raw
            stream = io.BytesIO()
            with self.assertRaises(ValueError):
                voice.speak("queen", "The next complete sentence follows.", worker.AudioOut(stream))
            self.assertEqual(stream.getvalue(), struct.pack("<ii", 48_000, 0))
        self.assertEqual(worker.prepare_audio(np.array([], dtype=np.float32)).size, 0)
        np.testing.assert_array_equal(worker.prepare_audio(np.array([.1])), [0.0])

    def test_failed_encoding_ends_one_frame_and_next_request_stays_aligned(self):
        voice = self.voice()
        stream = io.BytesIO()
        voice._engine.encode_prompt.side_effect = RuntimeError("bad reference")
        with self.assertRaises(RuntimeError):
            voice.speak("queen", "First sentence.", worker.AudioOut(stream))
        self.assertEqual(stream.getvalue(), struct.pack("<ii", 48000, 0))
        voice._engine.encode_prompt.side_effect = None
        voice.speak("queen", "The next complete sentence follows.", worker.AudioOut(stream))
        stream.seek(8)
        self.assertEqual(struct.unpack("<ii", stream.read(8)), (48000, 480))
        stream.read(480 * 4)
        self.assertEqual(stream.read(), struct.pack("<i", 0))


if __name__ == "__main__":
    unittest.main()

import unittest

import get_youtube_transcript as yt


class YouTubeTranscriptHelpersTest(unittest.TestCase):
    def test_extracts_supported_youtube_urls(self):
        video_id = "dQw4w9WgXcQ"
        cases = [
            video_id,
            f"https://www.youtube.com/watch?v={video_id}&t=20",
            f"https://youtu.be/{video_id}?si=test",
            f"https://www.youtube.com/shorts/{video_id}",
            f"https://www.youtube.com/live/{video_id}?feature=share",
            f"https://www.youtube.com/embed/{video_id}",
        ]
        for value in cases:
            with self.subTest(value=value):
                self.assertEqual(yt.video_id_from_url(value), video_id)

    def test_rejects_unrelated_or_malformed_urls(self):
        for value in [
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=short",
            "not-a-video-id",
        ]:
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    yt.video_id_from_url(value)

    def test_formats_timestamp_evidence(self):
        self.assertEqual(yt.stamp(0), "00:00")
        self.assertEqual(yt.stamp(197.9), "03:17")
        self.assertEqual(yt.stamp(3723), "01:02:03")


if __name__ == "__main__":
    unittest.main()

import copy
import json
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pymusic_score as score
import teledra_synth as synth
from python_music_worker import render_source


def song_fixture():
    def events(positions, degrees, lengths):
        return [dict(beat=beat, degree=degree, length=length, velocity=.8)
                for beat, degree, length in zip(positions, degrees, lengths)]
    return dict(
        title="Four rooms with an answer", tonic="D", mode="minor", bpm=112,
        progression=[0, 3, 4, 0],
        voices=dict(lead="classic:keys", answer="classic:bell", harmony="classic:pad",
                    bass="classic:bass", texture="classic:pluck"),
        theme=events([0, 1, 2.5, 4, 5.25, 6.5], [0, 2, 1, 4, 2, 0], [.5, .75, .5, 1, .5, .75]),
        answer=events([.5, 2, 3, 4.5, 6], [2, 1, 0, 4, 0], [.5, .5, .75, 1, 1]),
        sections=[dict(name=name, bars=4, melody=melody, energy=energy)
                  for name, melody, energy in [("Opening", "theme", .55), ("Answer", "answer", .72),
                                               ("Contrast", "variation", .9), ("Return", "theme", .65)]],
        pulse="syncopated",
    )


def source_for(song, sr=8000, ending=None):
    return ("from teledra_synth import render_song, play_sound\n"
            f"SONG = {song!r}\n"
            f"audio = render_song(SONG, sr={sr})\n"
            + (ending or f"play_sound(audio, sr={sr}, loop=True)\n"))


class SongRendererTests(unittest.TestCase):
    def tearDown(self):
        synth._capture_callback = None
        synth._close_bridge()

    def test_full_render_has_four_measured_sections_five_layers_and_complete_ledger(self):
        audio, report = render_source(source_for(song_fixture()))
        audit = report["composition_audit"]
        self.assertEqual((audit["layer_count"], audit["section_count"], audit["active_layers"]), (5, 4, 5))
        self.assertEqual(audit["notes_scheduled"], audit["notes_mixed"])
        self.assertEqual(audit["notes_mixed"], len(audit["events"]))
        self.assertEqual(audit["renderer"], "render_song")
        self.assertTrue(audit["complete"])
        self.assertGreaterEqual(report["duration_seconds"], 30)
        self.assertTrue(np.all(np.isfinite(audio)))
        self.assertLessEqual(report["peak"], .90)
        self.assertEqual(audio.shape[1], 2)
        self.assertGreater(report["rms"], .001)
        self.assertEqual({event["id"] for event in audit["events"]}, set(range(audit["notes_mixed"])))
        self.assertTrue(all(event["mixed"] for event in audit["events"]))
        for layer in audit["layer_stats"]:
            self.assertGreater(layer["events"], 0)
            self.assertGreater(layer["rms"], 1e-6)
        for section in audit["sections"]:
            self.assertGreater(section["rms"], .001)
            self.assertGreaterEqual(len(section["active_layers"]), 4)
        self.assertGreater(audit["sections"][2]["rms"], audit["sections"][0]["rms"] * 1.25)
        self.assertEqual(report["metadata"]["bars"], 16)
        self.assertEqual(report["metadata"]["key"], "D minor")
        self.assertAlmostEqual(audio[0, 0], audio[-1, 0], places=12)
        # Classic colour recipes can synthesize multiple components for one
        # placed note; the helper count is deliberately not the event count.
        self.assertGreater(report["notes_rendered"], audit["notes_mixed"])

    def test_every_scheduled_event_contributes_once_to_the_actual_additive_mix(self):
        song = song_fixture()
        song["voices"] = {role: "classic:sine" for role in score.ROLES}
        recorded = []
        def known_note(_freq, duration, **kwargs):
            note = np.full(int(duration * kwargs["sr"]), .01 + len(recorded) * .00001)
            recorded.append(note)
            return note
        with patch.object(synth, "synth_note", side_effect=known_note):
            actual, audit = score.render(song, 8000, synth)
        expected = np.zeros_like(actual)
        for event, note in zip(audit["events"], recorded):
            angle = (score.PAN[event["layer"]] + 1) * np.pi / 4
            start = int(event["start_seconds"] * 8000)
            note = note * score.LEVEL[event["layer"]]
            expected[start:start + len(note), 0] += note * np.cos(angle)
            expected[start:start + len(note), 1] += note * np.sin(angle)
        expected = synth.make_seamless_loop(synth._protect_playback_peak(expected, .90), .015, sr=8000)
        self.assertEqual(len(recorded), audit["notes_scheduled"])
        np.testing.assert_allclose(actual, expected, atol=1e-14)

    def test_planned_parts_have_independent_rhythm_and_real_phrase_end_gaps(self):
        events, _ = score.plan_song(song_fixture())
        lead = [event for event in events if event["layer"] == "lead"]
        answer = [event for event in events if event["layer"] == "answer"]
        self.assertNotEqual({event["start_beat"] for event in lead}, {event["start_beat"] for event in answer})
        for role in ("lead", "answer"):
            for phrase in range(8):
                part = [event for event in events if event["layer"] == role and int(event["start_beat"] // 8) == phrase]
                if part:
                    end = max(event["start_beat"] + event["length_beats"] for event in part)
                    self.assertLessEqual(end, phrase * 8 + 7.75)
        # Support is independently timed; it cannot merely double all melody slots.
        bass_times = {event["start_beat"] for event in events if event["layer"] == "bass"}
        self.assertNotEqual(bass_times, {event["start_beat"] for event in lead + answer})

    def test_whole_part_register_fit_preserves_contour_and_strong_beat_harmony(self):
        song = song_fixture()
        events, _ = score.plan_song(song)
        scale = score.MODES[song["mode"]]
        tonic = score.TONICS.index(song["tonic"])
        for event in events:
            if event["layer"] in ("lead", "answer") and event["start_beat"] % 2 == 0:
                root = song["progression"][int(event["start_beat"] // 4) % 4]
                chord = {score._midi(root + interval, tonic, scale) % 12 for interval in (0, 2, 4)}
                self.assertIn(event["midi"] % 12, chord)
        catalog = [{"id": "test_voice", "range": [48, 100]}]
        song["voices"]["lead"] = "test_voice"
        shifted, _ = score.plan_song(song, catalog)
        old = [e["midi"] for e in events if e["layer"] == "lead"]
        new = [e["midi"] for e in shifted if e["layer"] == "lead"]
        self.assertEqual(len(set(b - a for a, b in zip(old, new))), 1)

    def test_invalid_and_scalar_plans_fail_before_any_synthesis(self):
        mutations = [
            lambda s: s.update(bpm=132),
            lambda s: s.update(progression=[0, 0, 0, 0]),
            lambda s: s.update(answer=copy.deepcopy(s["theme"])),
            lambda s: s["voices"].update(lead="classic:not_a_voice"),
            lambda s: s["theme"][1].update(beat=0),
            lambda s: s["theme"][-1].update(beat=7.5, length=1),
            lambda s: [event.update(degree=i) for i, event in enumerate(s["theme"])],
            lambda s: s.update(sections=[dict(name=str(i), bars=4, melody="theme", energy=.6) for i in range(4)]),
            lambda s: [section.update(energy=.6) for section in s["sections"]],
            lambda s: s.update(extra="not allowed"),
        ]
        for mutation in mutations:
            song = song_fixture()
            mutation(song)
            with self.subTest(mutation=mutation), patch.object(synth, "synth_note", side_effect=AssertionError("must validate first")):
                with self.assertRaises(ValueError):
                    score.render(song, 8000, synth)

    def test_render_is_deterministic_and_musical_recipe_changes_change_pcm(self):
        song = song_fixture()
        first, first_audit = score.render(song, 8000, synth)
        second, second_audit = score.render(song, 8000, synth)
        np.testing.assert_array_equal(first, second)
        self.assertEqual(first_audit, second_audit)
        song["answer"][1]["degree"] = 3
        different, _ = score.render(song, 8000, synth)
        self.assertFalse(np.array_equal(first, different))

    def test_same_native_piano_keeps_bass_harmony_and_lead_in_separate_registers(self):
        song = song_fixture()
        song["voices"] = {role: "piano" for role in score.ROLES}
        events, _ = score.plan_song(song, [{"id": "piano", "range": [33, 91]}])
        parts = {role: [event["midi"] for event in events if event["layer"] == role] for role in score.ROLES}
        self.assertLessEqual(max(parts["bass"]), 52)
        self.assertGreaterEqual(min(parts["lead"]), 57)
        self.assertGreater(np.mean(parts["lead"]), np.mean(parts["bass"]) + 16)
        self.assertLess(np.mean(parts["bass"]), np.mean(parts["harmony"]) - 10)
        self.assertLess(np.mean(parts["harmony"]), np.mean(parts["lead"]) - 7)

    def test_authored_metadata_cannot_forge_audit_or_replace_the_captured_song(self):
        song = song_fixture()
        ending = "TELEDRA_SCORE = {'title':'forged','bars':999}\nTELEDRA_EVENTS = []\nplay_sound(audio, sr=8000)"
        _, report = render_source(source_for(song, ending=ending))
        self.assertEqual(report["title"], song["title"])
        self.assertEqual(report["metadata"]["bars"], 16)
        for ending in ("play_sound(audio[:32000], sr=8000)", "play_sound(audio * .5, sr=8000)",
                       "play_sound(audio, sr=44100)"):
            with self.subTest(ending=ending), self.assertRaisesRegex(ValueError, "replaced or modified"):
                render_source(source_for(song, ending=ending))
        bad = song_fixture()
        bad["theme"][0]["degree"] = 99
        source = ("from teledra_synth import render_song, play_sound\n"
                  f"try:\n    render_song({bad!r},sr=8000)\nexcept ValueError:\n    pass\n"
                  "play_sound(np.ones(32000)*.1,sr=8000)")
        with self.assertRaisesRegex(ValueError, "successfully render exactly one"):
            render_source(source)

    @unittest.skipUnless(os.environ.get("TELEDRA_SYNTH_EXE"), "requires built native note server")
    def test_all_72_catalog_voices_are_eligible_and_native_song_renders_completely(self):
        executable = os.environ["TELEDRA_SYNTH_EXE"]
        synth._configure_bridge(executable)
        catalog = synth.instruments()
        self.assertGreaterEqual(len(catalog), 72)
        for entry in catalog:
            song = song_fixture()
            song["voices"] = {role: entry["id"] for role in score.ROLES}
            events, _ = score.plan_song(song, catalog)
            self.assertTrue(events, entry["id"])
        synth._close_bridge()
        song = song_fixture()
        song["voices"] = dict(lead="piano", answer="clarinet", harmony="warm_pad", bass="double_bass", texture="shaker")
        _, report = render_source(source_for(song, sr=44100), synth_exe=executable)
        self.assertEqual(report["composition_audit"]["active_layers"], 5)
        self.assertEqual(report["notes_requested"], report["notes_rendered"])


if __name__ == "__main__":
    unittest.main()

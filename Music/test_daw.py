"""Exercise the real Tk workstation hidden, without starting audio or a court."""
import tkinter as tk
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from daw import OrganistStudio, chord_name, chord_cycle_state, expressed_steps, sequencer_column, sequencer_note, score_grid, PANEL


class StudioTests(unittest.TestCase):
    def test_engine_buttons_request_runtime_switch_and_alternation(self):
        def descendants(widget):
            for child in widget.winfo_children():
                yield child
                yield from descendants(child)

        buttons = {str(widget.cget("text")): widget for widget in descendants(self.root)
                   if widget.winfo_class() == "TButton"}
        with patch.object(self.studio, "send_action") as send:
            buttons["PyMusic"].invoke()
            buttons["Alternate: off"].invoke()
        self.assertEqual([call.args for call in send.call_args_list],
                         [("python_music",), ("toggle_alternation",)])
        self.assertEqual(self.studio.alternate_var.get(), "Alternate: off")
        self.studio.apply_command({"action": "alternating", "enabled": True})
        self.assertEqual(self.studio.alternate_var.get(), "Alternate: on")
        self.studio.apply_command({"action": "alternating", "enabled": "false"})
        self.assertEqual(self.studio.alternate_var.get(), "Alternate: on")

    def test_complete_arrangement_follows_later_written_pages(self):
        steps = [None] * 256
        steps[128] = 5
        score = {"bpm": 120, "bars": 32, "composition": {"steps_per_bar": 8},
                 "tracks": [{"name": "Late entry", "instrument": "piano", "gain": .5,
                             "steps": steps}]}
        with patch.object(self.studio, "send_action"):
            self.studio.apply_command({"action": "score", "score": score,
                                       "transport": "playing", "position_seconds": 32.0})
        self.studio.draw_sequencer()
        cell = self.studio.canvas.find_withtag("note_cell_0_0")[0]
        self.assertNotEqual(self.studio.canvas.itemcget(cell, "fill"), PANEL)
        labels = [self.studio.canvas.itemcget(item, "text") for item in self.studio.canvas.find_all()
                  if self.studio.canvas.type(item) == "text"]
        self.assertIn("17", labels)
        self.assertNotIn("1", labels)

    def test_written_eight_bar_grid_tracks_the_actual_phrase(self):
        score = {"bpm": 120, "bars": 8, "composition": {"steps_per_bar": 8}}
        self.assertEqual(score_grid(score), 8)
        # Eight seconds starts bar 5: the second half of a 64-slot melody must
        # not be displayed as another repeat of the first four bars.
        self.assertEqual(sequencer_column(score, 8.0, 64, []), 32)
        self.assertEqual(sequencer_note(list(range(64)), 32, 8), 32)
        self.assertEqual(sequencer_note([0, 2, 4, 0], 2, 8), 2)
        self.assertIsNone(sequencer_note([0, 2, 4, 0], 1, 8))
        score["composition"] = {"steps_per_bar": 4, "form": "binary"}
        self.assertEqual(sequencer_column(score, 8.0, 64, [("A", 0, 4), ("B", 4, 8)]), 0)

    def setUp(self):
        self.root = tk.Tk()
        self.root.withdraw()
        with patch.object(OrganistStudio, "read_commands"), patch("builtins.print"):
            self.studio = OrganistStudio(self.root)
        self.score = {"title": "A movement", "bpm": 96, "bars": 48, "tonic": 2, "mode": "dorian",
                      "tracks": [{"name": "Organ", "instrument": "organ", "gain": .4,
                                  "steps": [0, None, 2, None]}]}

    def tearDown(self):
        self.studio.closing = True
        self.root.destroy()

    def test_instrument_browser_shows_complete_catalog_searches_and_reloads_without_audio(self):
        schema = json.loads(Path(__file__).with_name("score.schema.json").read_text(encoding="utf-8"))
        ids = schema["properties"]["tracks"]["items"]["properties"]["instrument"]["enum"]
        self.assertGreaterEqual(len(ids), 70)
        catalog = [{"id": instrument, "name": instrument.replace("_", " ").title(),
                    "group": "Electronic" if instrument == "theremin" else "Other",
                    "role": "melody", "range": [48, 96], "description": "A singing phrase.",
                    "monophonic": instrument == "theremin"} for instrument in ids]
        with patch.object(self.studio, "send_action") as send:
            self.studio.apply_command({"action": "score", "score": self.score,
                                       "instrument_catalog": catalog, "transport": "paused",
                                       "position_seconds": 12.5})
            self.studio.open_catalog()
            self.studio.catalog_window.withdraw()
            self.root.update_idletasks()
            self.assertEqual(len(self.studio.catalog_tree.get_children()), len(ids))
            self.assertIn(str(len(ids)), self.studio.catalog_button_var.get())
            self.studio.catalog_search_var.set("theremin")
            self.assertEqual(self.studio.catalog_tree.get_children(), ("theremin",))
            self.assertIn("One melodic note at a time", self.studio.catalog_detail_var.get())
            self.assertIn("48–96", self.studio.catalog_detail_var.get())
            self.studio.catalog_search_var.set("piano")
            self.assertEqual(set(self.studio.catalog_tree.get_children()),
                             {"piano", "electric_piano", "honky_tonk_piano"})
            self.studio.catalog_search_var.set("electronic")
            self.assertEqual(self.studio.catalog_tree.get_children(), ("theremin",))
            self.studio.catalog_search_var.set("no matching voice")
            self.assertEqual(self.studio.catalog_tree.get_children(), ())
            self.assertIn("No voices match", self.studio.catalog_detail_var.get())
            self.studio.catalog_search_var.set("")
            self.studio.refresh_catalog()
            self.assertEqual(len(self.studio.catalog_tree.get_children()), len(ids))
            self.assertEqual(self.studio.transport, "paused")
            self.assertEqual(self.studio.position_seconds, 12.5)
            send.assert_not_called()
            self.studio.catalog_window.destroy()

    def test_catalog_ignores_malformed_and_duplicate_entries(self):
        voice = {"id": "theremin", "name": "Theremin", "group": "Electronic",
                 "role": "melody", "range": [48, 96]}
        self.studio.apply_command({"action": "score", "score": self.score,
                                   "instrument_catalog": [None, {}, voice, voice,
                                        {**voice, "id": "bad", "range": [96, 48]}]})
        self.assertEqual(self.studio.instrument_catalog, [voice])

    def test_cadence_transport_and_mute_survive_hot_reload(self):
        self.studio.apply_command({"action": "score", "score": self.score, "transport": "muted", "cadence_seconds": 600})
        self.assertEqual(self.studio.cadence_seconds, 600)
        self.assertIn("10 min", self.studio.cadence_var.get())
        self.studio.last_clock -= 2
        self.studio.advance_clock()
        self.assertGreater(self.studio.position_seconds, 1.9)
        self.studio.apply_command({"action": "transport", "state": "paused"})
        paused = self.studio.position_seconds
        self.studio.last_clock -= 2
        self.studio.advance_clock()
        self.assertEqual(self.studio.position_seconds, paused)
        self.studio.muted_gains[0] = .4
        changed = {**self.score, "title": "A movement // Movement 4: Texture",
                   "tracks": [{**self.score["tracks"][0], "name": "Reed", "instrument": "reed", "gain": 0}]}
        self.studio.apply_command({"action": "score", "score": changed, "transport": "paused", "cadence_seconds": 250})
        self.assertEqual(self.studio.muted_gains[0], .4)
        self.assertEqual(self.studio.transport, "paused")
        self.assertIn("250s", self.studio.cadence_var.get())
        self.root.update_idletasks()

    def test_muting_does_not_restart_the_development_countdown(self):
        self.studio.apply_command({"action": "score", "score": self.score, "transport": "playing", "cadence_seconds": 300})
        self.studio.next_change_at -= 43
        deadline = self.studio.next_change_at
        self.studio.apply_command({"action": "transport", "state": "muted"})
        self.studio.apply_command({"action": "transport", "state": "playing"})
        self.assertEqual(self.studio.next_change_at, deadline)

    def test_mixer_reload_preserves_native_playhead_and_pending_movement_status(self):
        self.studio.apply_command({"action": "score", "score": self.score,
                                   "transport": "paused", "position_seconds": 37.25})
        self.assertEqual(self.studio.position_seconds, 37.25)
        self.studio.apply_command({"action": "status", "message": "Next movement ready · finishing the current piece"})
        self.assertIn("finishing", self.studio.status_var.get())
        self.assertEqual(self.studio.position_seconds, 37.25)
        self.studio.apply_command({"action": "score", "score": self.score,
                                   "transport": "paused"})
        self.assertEqual(self.studio.position_seconds, 0.0)
        self.assertNotIn("finishing", self.studio.status_var.get())

    def test_theory_and_actual_form_refresh_without_reopening_the_studio(self):
        theory = {"key": "D harmonic minor", "progression": [{"roman": "i"}, {"roman": "V7"}],
                  "motif": {"track": "Flute theme", "contour": "ascending"},
                  "next_experiment": "Invert the answer while keeping the opening."}
        timeline = [["A", 0, 16], ["B", 16, 32], ["A", 32, 48]]
        self.studio.apply_command({"action": "score", "score": self.score,
                                   "theory": theory, "form_timeline": timeline})
        self.assertIn("V7", self.studio.theory_var.get())
        self.assertIn("Invert the answer", self.studio.experiment_var.get())
        self.assertEqual(self.studio.form_timeline, [("A", 0, 16), ("B", 16, 32), ("A", 32, 48)])
        self.assertEqual(len(self.studio.timeline.find_withtag("form_label")), 3)
        theory["next_experiment"] = "Retain common tones through the chord changes."
        self.studio.apply_command({"action": "score", "score": self.score,
                                   "theory": theory, "form_timeline": [["A", 0, 24], ["B", 24, 48]]})
        self.assertIn("Retain common tones", self.studio.experiment_var.get())
        self.assertEqual(len(self.studio.timeline.find_withtag("form_label")), 2)
        self.assertNotIn("Invert", self.studio.experiment_var.get())

    def test_harmony_highlight_repeats_at_the_authored_chord_duration(self):
        score = {**self.score, "bpm": 120, "composition": {"chord_bars": 2},
                 "progression": [{"root_degree": 0, "quality": "minor_seventh"},
                                 {"root_degree": 4, "quality": "dominant_seventh"}]}
        self.assertEqual(chord_cycle_state(score, 0), (0, 0, 2, 4))
        self.assertEqual(chord_cycle_state(score, 4), (1, 2, 2, 4))
        self.assertEqual(chord_cycle_state(score, 8), (0, 4, 2, 4))
        self.studio.apply_command({"action": "score", "score": score})
        self.studio.position_seconds = 4.0
        self.studio.draw_timeline()
        current = self.studio.timeline.find_withtag("current_chord")
        self.assertEqual(len(current), 1)
        self.assertIn("A7", self.studio.timeline.itemcget(current[0], "text"))
        self.assertIn("4-BAR CYCLE", self.studio.timeline.itemcget(
            self.studio.timeline.find_withtag("harmonic_rhythm")[0], "text"))

    def test_form_return_uses_renderer_harmony_indices(self):
        score = {**self.score, "bars": 12, "bpm": 120, "composition": {"chord_bars": 1},
                 "progression": [{"root_degree": degree, "quality": "major"}
                                 for degree in [0, 3, 4]]}
        # A phrase starts anew at bar 5; a global modulo would incorrectly show IV.
        harmony = [0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0]
        self.studio.apply_command({"action": "score", "score": score, "harmonic_timeline": harmony})
        self.studio.position_seconds = 8.0
        self.studio.draw_timeline()
        self.assertEqual(chord_cycle_state(score, 8.0, harmony), (0, 4, 1, 3))
        current = self.studio.timeline.find_withtag("current_chord")[0]
        self.assertEqual(self.studio.timeline.itemcget(current, "text"), "▶ D")

    def test_zero_velocity_note_is_a_rest_in_sequencer_and_activity_meter(self):
        track = {**self.score["tracks"][0], "steps": [0, 5, 2, None],
                 "velocities": [0.8, 0.0, 0.7, 0.0]}
        score = {**self.score, "bpm": 120, "tracks": [track]}
        self.studio.apply_command({"action": "score", "score": score, "transport": "playing"})
        self.studio.position_seconds = 0.5
        self.studio.draw_sequencer()
        self.assertEqual(expressed_steps(track), [0, None, 2, None])
        self.assertEqual(track["steps"], [0, 5, 2, None])
        cell = self.studio.canvas.find_withtag("note_cell_0_4")[0]
        self.assertEqual(self.studio.canvas.itemcget(cell, "fill"), PANEL)
        meter = self.studio.canvas.coords(self.studio.canvas.find_withtag("activity_meter_0")[0])
        self.assertEqual(meter[0], meter[2])


class TheoryLabelTests(unittest.TestCase):
    def test_multibar_pattern_restarts_at_exact_form_boundaries(self):
        score = {"bpm": 120, "composition": {"form": "ternary"}}
        timeline = [("A", 0, 2), ("B", 2, 6), ("A", 6, 8)]
        self.assertEqual(sequencer_column(score, 3.5, 64, timeline), 28)
        self.assertEqual(sequencer_column(score, 4.0, 64, timeline), 0)
        self.assertEqual(sequencer_column(score, 12.0, 64, timeline), 0)
        self.assertEqual(sequencer_column({"bpm": 120}, 4.0, 64, timeline), 32)

    def test_new_modes_and_seventh_qualities_use_their_actual_pitch_classes(self):
        cases = [
            ("lydian", 3, "major", "F#"),
            ("phrygian", 1, "major", "C#"),
            ("harmonic_minor", 6, "diminished", "B°"),
            ("major", 0, "major_seventh", "Cmaj7"),
            ("major", 1, "minor_seventh", "Dm7"),
            ("major", 4, "dominant_seventh", "G7"),
            ("minor", 1, "half_diminished", "Dø7"),
            ("harmonic_minor", 2, "augmented", "D#+"),
        ]
        for mode, degree, quality, expected in cases:
            with self.subTest(mode=mode, quality=quality):
                self.assertEqual(chord_name({"tonic": 0, "mode": mode},
                    {"root_degree": degree, "quality": quality}), expected)


if __name__ == "__main__":
    unittest.main()

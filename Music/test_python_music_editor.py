"""Exercise the actual Python authoring window hidden, without executing music."""
import io
import json
from pathlib import Path
import tkinter as tk
import unittest

from python_music_editor import PythonMusicEditor, catalog_entries, library_entries, library_saved_time, protocol_action


class PythonEditorTests(unittest.TestCase):
    def setUp(self):
        self.root = tk.Tk()
        self.root.withdraw()
        self.events = []
        self.studio = PythonMusicEditor(self.root, start_io=False, emit=self.events.append)
        schema = json.loads(Path(__file__).with_name("score.schema.json").read_text(encoding="utf-8"))
        self.ids = schema["properties"]["tracks"]["items"]["properties"]["instrument"]["enum"]
        self.catalog = [{"id": name, "name": name.replace("_", " ").title(),
                         "range": [48, 96], "role": "melody", "group": "Electronic",
                         "attack_spacing": 0.5, "monophonic": name == "theremin",
                         "description": "A spacious singing line."} for name in self.ids]

    def tearDown(self):
        self.studio.close(notify=False)

    def load_source(self, code="full_track = None\n"):
        self.studio.apply_command({"action": "source", "code": code,
                                   "transport": "paused", "instrument_catalog": self.catalog,
                                   "alternating": True})

    def test_all_72_instruments_are_searchable_and_insert_exact_api_without_playing(self):
        self.load_source()
        self.assertGreaterEqual(len(self.ids), 70)
        self.assertEqual(set(self.studio.catalog_tree.get_children()), set(self.ids))
        self.studio.catalog_search_var.set("theremin")
        self.assertEqual(self.studio.catalog_tree.get_children(), ("theremin",))
        self.assertIn("One melodic note at a time", self.studio.catalog_detail_var.get())
        self.assertIn("48–96", self.studio.catalog_detail_var.get())
        self.assertIn('instrument="theremin"', self.studio.snippet_var.get())
        self.assertIn("instrument_note", self.studio.snippet_var.get())
        self.studio.insert_instrument()
        self.assertIn('instrument="theremin"', self.studio.code())
        self.studio.catalog_search_var.set("piano")
        self.assertEqual(set(self.studio.catalog_tree.get_children()),
                         {"piano", "electric_piano", "honky_tonk_piano"})
        self.studio.catalog_search_var.set("no matching instrument")
        self.assertEqual(self.studio.catalog_tree.get_children(), ())
        self.assertEqual(self.studio.snippet_var.get(), "")
        self.assertEqual(self.studio.transport, "paused")
        self.assertEqual(self.events, [{"event": "ready", "protocol": 1}])

    def test_run_and_save_submit_exact_source_without_executing_or_changing_transport(self):
        code = '# Unicode phrase — not executed\nraise RuntimeError("never execute here")\n'
        self.load_source(code)
        self.studio.buttons["run"].invoke()
        self.studio.buttons["save"].invoke()
        self.assertEqual(self.events[-2:], [{"action": "run", "code": code},
                                           {"action": "save", "code": code}])
        self.assertEqual(self.studio.transport, "paused")
        self.studio.apply_command({"action": "status", "message": "Render rejected; source retained"})
        self.assertEqual(self.studio.code(), code)
        self.assertIn("rejected", self.studio.status_var.get())

    def test_all_controls_request_runtime_actions_and_wait_for_transport_confirmation(self):
        self.load_source()
        self.assertEqual(self.studio.alternate_var.get(), "Alternate: on")
        controls = ("play", "pause", "stop", "native", "compose", "alternate", "keep", "reject")
        for action in controls:
            self.studio.buttons[action].invoke()
        expected = []
        for action in controls:
            if action in {"native", "compose"}:
                expected.append({"action": "save", "code": self.studio.code()})
            expected.append({"action": action})
        self.assertEqual(self.events[1:], expected)
        self.assertEqual(self.studio.transport_var.get(), "PAUSED")
        self.studio.apply_command({"action": "transport", "state": "playing"})
        self.assertEqual(self.studio.transport_var.get(), "PLAYING")
        self.studio.apply_command({"action": "transport", "state": "shell"})
        self.assertEqual(self.studio.transport, "playing")
        self.studio.apply_command({"action": "alternating", "enabled": False})
        self.assertEqual(self.studio.alternate_var.get(), "Alternate: off")

    def test_hot_source_updates_keep_unsaved_edits_until_explicit_reload(self):
        self.load_source("accepted = 1\n")
        self.studio.editor.insert("end", "my_edit = 2\n")
        edited = self.studio.code()
        self.studio.apply_command({"action": "source", "code": "accepted = 3\n",
                                   "transport": "muted", "alternating": False})
        self.assertEqual(self.studio.code(), edited)
        self.assertIn("edits are retained", self.studio.status_var.get())
        self.assertEqual(self.studio.transport, "muted")
        self.studio.reload_source()
        self.assertEqual(self.studio.code(), "accepted = 3\n")
        self.assertEqual(self.studio.alternate_var.get(), "Alternate: off")

    def test_close_saves_exact_draft_before_closing_once(self):
        self.load_source("draft = 4\n")
        self.studio.close()
        self.studio.close()
        self.assertEqual(self.events[-2:], [{"action": "save", "code": "draft = 4\n"},
                                           {"action": "close"}])
        self.assertTrue(self.studio.closing)
        self.assertIsNone(self.studio.after_id)

    def test_stdin_eof_closes_window_and_saves_last_draft(self):
        messages = ('not-json\n[]\n' + json.dumps({"action": "source", "code": "last = 5\n"}) + '\n')
        self.studio.read_commands(io.StringIO(messages))
        self.studio.tick()
        self.assertTrue(self.studio.closing)
        self.assertEqual(self.events[-2:], [{"action": "save", "code": "last = 5\n"},
                                           {"action": "close"}])

    def test_unknown_commands_and_malformed_catalog_do_not_execute(self):
        self.studio.apply_command({"action": "execute", "code": "raise RuntimeError()"})
        self.studio.apply_command({"action": "source", "code": None, "alternating": "yes"})
        self.assertEqual(self.studio.code(), "")
        self.assertFalse(self.studio.alternating)
        valid = self.catalog[0]
        self.assertEqual(catalog_entries([None, {}, valid, valid,
                                          {**valid, "id": "invalid", "range": [96, 48]}]), [valid])
        for action, fields in (("shell", {"code": "x"}), ("play", {"code": "x"}),
                               ("run", {}), ("save", {"code": 7})):
            with self.assertRaises(ValueError):
                protocol_action(action, **fields)

    def test_library_lists_verdicts_and_requests_only_known_id_without_changing_draft(self):
        self.load_source("working_draft = 4\n")
        original = self.studio.code()
        entries = [
            {"id": "py-0123456789abcdef", "title": "First complete piece", "duration_seconds": 88,
             "note_count": 754, "verdict": "unreviewed"},
            {"id": "py-fedcba9876543210", "title": "Quiet reply", "duration_seconds": 75,
             "note_count": 640, "verdict": "keep"}]
        self.studio.apply_command({"action": "library", "entries": entries, "active_id": entries[0]["id"]})
        self.assertEqual(set(self.studio.library_tree.get_children()), {item["id"] for item in entries})
        self.assertIn("Currently selected", self.studio.library_detail_var.get())
        self.studio.library_tree.selection_set(entries[1]["id"])
        self.studio.show_library_selection()
        self.assertIn("Listener verdict: keep", self.studio.library_detail_var.get())
        self.studio.library_play_button.invoke()
        self.assertEqual(self.events[-2:], [{"action": "save", "code": original},
                                           {"action": "library_play", "id": entries[1]["id"]}])
        self.assertEqual(self.studio.code(), original)
        self.assertEqual(self.studio.transport, "paused")
        self.studio.buttons["keep"].invoke()
        self.assertEqual(self.events[-1], {"action": "keep"}, "verdict stays bound to runtime active audio")
        self.studio.apply_command({"action": "source", "library": entries, "active_library_id": entries[1]["id"]})
        self.assertEqual(self.studio.active_library_id, entries[1]["id"])
        self.assertEqual(self.studio.code(), original)

    def test_library_malformed_rows_and_paths_cannot_become_play_requests(self):
        valid = {"id": "py-0123456789abcdef", "title": "Stored piece", "duration_seconds": 60,
                 "note_count": 64, "verdict": "unreviewed"}
        malformed = [None, {}, valid, valid, {**valid, "id": "../audio.wav"},
                     {**valid, "duration_seconds": float("nan")}, {**valid, "duration_seconds": 4},
                     {**valid, "note_count": 0}, {**valid, "verdict": "approved"}]
        self.assertEqual(library_entries(malformed), [valid])
        self.studio.apply_command({"action": "library", "entries": [], "active_id": "../audio.wav"})
        self.studio.play_library()
        self.assertEqual(self.events, [{"event": "ready", "protocol": 1}])
        for fields in ({"id": "../audio.wav"}, {"id": valid["id"], "path": "audio.wav"}, {}, {"id": 7}):
            with self.assertRaises(ValueError):
                protocol_action("library_play", **fields)

    def test_duplicate_title_labels_and_saved_time_keep_playback_bound_to_original_id(self):
        base = {"title": "Evolving Harmony", "duration_seconds": 35.56, "note_count": 150,
                "verdict": "unreviewed"}
        first = {**base, "id": "py-0123456789abcdef", "display_title": "Evolving Harmony · v1",
                 "added_ms": 1789343032523}
        second = {**base, "id": "py-fedcba9876543210", "display_title": "Evolving Harmony · v2",
                  "added_ms": 1789343103711}
        self.studio.apply_command({"action": "library", "entries": [second, first], "active_id": second["id"]})
        self.assertEqual(self.studio.library_tree.item(first["id"], "text"), "Evolving Harmony · v1")
        self.assertEqual(self.studio.library_tree.item(second["id"], "text"), "▶ Evolving Harmony · v2")
        self.assertIn(library_saved_time(second), self.studio.library_detail_var.get())
        self.assertRegex(library_saved_time(second), r"^Saved \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\n$")
        self.studio.library_tree.selection_set(first["id"])
        self.studio.show_library_selection()
        self.studio.library_play_button.invoke()
        self.assertEqual(self.events[-1], {"action": "library_play", "id": first["id"]})
        self.assertEqual(self.studio.library[0]["title"], "Evolving Harmony")
        self.assertEqual(library_entries([{**first, "display_title": 4}, {**first, "added_ms": -1},
                                          {**first, "added_ms": True}, first]), [first])
        self.assertEqual(library_saved_time(base), "")


if __name__ == "__main__":
    unittest.main()

"""Python music authoring window; playback and source persistence belong to Rust.

Newline-delimited JSON on stdin updates source/transport/status. JSON on stdout
requests an operation. This module never executes source or opens an audio device.
"""
from __future__ import annotations

from datetime import datetime
import json
import math
import queue
import sys
import threading
import tkinter as tk
from tkinter import ttk
from typing import Callable, TextIO

BACKGROUND = "#060910"
PANEL = "#0e1622"
TEXT = "#eef3f8"
MUTED = "#8291a6"
CYAN = "#6ee7df"
GOLD = "#e6bd6a"
ACTIONS = {"run", "save", "play", "pause", "stop", "native", "compose",
           "alternate", "keep", "reject", "close", "library_play"}
TRANSPORTS = {"playing", "paused", "stopped", "muted", "rendering"}


def protocol_action(action: str, **fields: object) -> dict[str, object]:
    if action not in ACTIONS:
        raise ValueError(f"unsupported editor action: {action}")
    if action in {"run", "save"}:
        if set(fields) != {"code"} or not isinstance(fields["code"], str):
            raise ValueError(f"{action} requires source code")
    elif action == "library_play":
        if set(fields) != {"id"} or not valid_library_id(fields["id"]):
            raise ValueError("library_play requires a catalog ID")
    elif fields:
        raise ValueError(f"{action} does not accept fields")
    return {"action": action, **fields}


def valid_library_id(value: object) -> bool:
    return (isinstance(value, str) and len(value) == 19 and value.startswith("py-")
            and all(char in "0123456789abcdef" for char in value[3:]))


def library_entries(value: object) -> list[dict]:
    """Display catalog facts only; filenames and executable source are not inputs."""
    entries = []
    seen = set()
    for entry in value if isinstance(value, list) else []:
        if not isinstance(entry, dict):
            continue
        identifier = entry.get("id")
        seconds = entry.get("duration_seconds")
        if (not valid_library_id(identifier) or identifier in seen
                or not isinstance(entry.get("title"), str)
                or type(seconds) not in (int, float) or not math.isfinite(seconds)
                or not 30 <= seconds <= 600
                or type(entry.get("note_count")) is not int or entry["note_count"] < 1
                or entry.get("verdict") not in {"unreviewed", "keep", "reject"}
                or ("display_title" in entry and (not isinstance(entry["display_title"], str)
                    or not entry["display_title"].strip() or len(entry["display_title"]) > 200))
                or ("added_ms" in entry and (type(entry["added_ms"]) is not int
                    or not 0 <= entry["added_ms"] <= 253402300799999))):
            continue
        seen.add(identifier)
        entries.append(entry)
    return entries


def library_saved_time(entry: dict) -> str:
    if "added_ms" not in entry:
        return ""
    try:
        saved = datetime.fromtimestamp(entry["added_ms"] / 1000).strftime("%Y-%m-%d %H:%M:%S")
        return f"Saved {saved}\n"
    except (OSError, OverflowError, ValueError):
        return ""


def catalog_entries(value: object) -> list[dict]:
    """Retain valid unique catalog entries, including every supplied voice."""
    entries = []
    seen = set()
    if not isinstance(value, list):
        return entries
    for info in value:
        if not isinstance(info, dict):
            continue
        identifier = info.get("id")
        bounds = info.get("range")
        if (not isinstance(identifier, str) or not identifier
                or not all(char.isascii() and (char.isalnum() or char == "_") for char in identifier)
                or identifier in seen or not isinstance(bounds, list) or len(bounds) != 2
                or not all(type(note) is int for note in bounds)
                or not 0 <= bounds[0] <= bounds[1] <= 127):
            continue
        seen.add(identifier)
        entries.append(info)
    return entries


def instrument_snippet(info: dict) -> str:
    low, high = info["range"]
    midi = max(low, min(high, 60))
    names = ("C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B")
    note = f"{names[midi % 12]}{midi // 12 - 1}"
    return (f'note = instrument_note({json.dumps(note)}, 0.5, '
            f'instrument={json.dumps(info["id"])}, volume=0.2, sr=44100)\n')


class PythonMusicEditor:
    def __init__(self, root: tk.Tk, *, start_io: bool = True,
                 emit: Callable[[dict], None] | None = None) -> None:
        self.root = root
        self.commands: queue.Queue[dict] = queue.Queue()
        self.emit = emit or self.write_event
        self.source = ""
        self.transport = "stopped"
        self.alternating = False
        self.instrument_catalog: list[dict] = []
        self.library: list[dict] = []
        self.active_library_id = None
        self.closing = False
        self.after_id = None
        root.title("Organist Studio // Python Music")
        root.geometry("1220x800+18+88")
        root.minsize(940, 600)
        root.configure(bg=BACKGROUND)
        root.protocol("WM_DELETE_WINDOW", self.close)
        self.build_window()
        root.bind("<Control-Return>", lambda _event: self.submit_source("run"))
        root.bind("<Control-s>", lambda _event: self.submit_source("save"))
        root.bind("<F5>", lambda _event: self.submit_source("run"))
        if start_io:
            threading.Thread(target=self.read_commands, daemon=True).start()
        self.emit({"event": "ready", "protocol": 1})
        self.after_id = root.after(40, self.tick)

    @staticmethod
    def write_event(event: dict) -> None:
        print(json.dumps(event, ensure_ascii=False), flush=True)

    def build_window(self) -> None:
        style = ttk.Style(self.root)
        style.theme_use("clam")
        style.configure("PyMusic.Treeview", background=PANEL, fieldbackground=PANEL,
                        foreground=TEXT, rowheight=24, borderwidth=0)
        style.configure("PyMusic.Treeview.Heading", background=BACKGROUND, foreground=MUTED)
        style.map("PyMusic.Treeview", background=[("selected", "#315773")],
                  foreground=[("selected", TEXT)])
        header = tk.Frame(self.root, bg=BACKGROUND, padx=18, pady=14)
        header.pack(fill="x")
        tk.Label(header, text="PYTHON MUSIC", bg=BACKGROUND, fg=GOLD,
                 font=("Consolas", 20, "bold")).pack(side="left")
        self.transport_var = tk.StringVar(value="STOPPED")
        tk.Label(header, textvariable=self.transport_var, bg=PANEL, fg=CYAN,
                 padx=14, pady=8, font=("Consolas", 11, "bold")).pack(side="right")

        toolbar = tk.Frame(self.root, bg=PANEL, padx=12, pady=8)
        toolbar.pack(fill="x", padx=18)
        self.buttons: dict[str, tk.Button] = {}
        for label, action in (("Run", "run"), ("Save", "save"), ("Play", "play"),
                              ("Pause", "pause"), ("Stop", "stop"),
                              ("Native Synth", "native"), ("New Music", "compose"),
                              ("Keep", "keep"), ("Reject", "reject")):
            callback = (lambda choice=action: self.submit_source(choice)) if action in {"run", "save"} else (
                lambda choice=action: self.control_action(choice))
            button = tk.Button(toolbar, text=label, command=callback, bg=PANEL, fg=TEXT,
                               activebackground="#21364b", activeforeground=TEXT,
                               relief="flat", padx=9, pady=5)
            button.pack(side="left", padx=2)
            self.buttons[action] = button
        self.alternate_var = tk.StringVar(value="Alternate: off")
        self.buttons["alternate"] = tk.Button(
            toolbar, textvariable=self.alternate_var, command=lambda: self.send_action("alternate"),
            bg=PANEL, fg=CYAN, activebackground="#21364b", activeforeground=TEXT,
            relief="flat", padx=9, pady=5)
        self.buttons["alternate"].pack(side="right")

        body = tk.PanedWindow(self.root, orient="horizontal", bg=BACKGROUND,
                              sashwidth=6, bd=0)
        body.pack(fill="both", expand=True, padx=18, pady=12)
        writing = tk.Frame(body, bg=PANEL)
        body.add(writing, minsize=510, stretch="always")
        document = tk.Frame(writing, bg=PANEL)
        document.pack(fill="x", padx=10, pady=6)
        self.document_var = tk.StringVar(value="Waiting for Python source")
        tk.Label(document, textvariable=self.document_var, bg=PANEL, fg=MUTED,
                 anchor="w").pack(side="left", fill="x", expand=True)
        tk.Button(document, text="Reload", command=self.reload_source, bg=PANEL, fg=TEXT,
                  relief="flat").pack(side="right")
        editor_box = tk.Frame(writing, bg=PANEL)
        editor_box.pack(fill="both", expand=True)
        self.editor = tk.Text(editor_box, undo=True, wrap="none", bg="#080e17", fg=TEXT,
                              insertbackground=CYAN, selectbackground="#315773",
                              font=("Consolas", 11), padx=12, pady=10, borderwidth=0)
        vertical = ttk.Scrollbar(editor_box, command=self.editor.yview)
        horizontal = ttk.Scrollbar(editor_box, orient="horizontal", command=self.editor.xview)
        self.editor.configure(yscrollcommand=vertical.set, xscrollcommand=horizontal.set)
        vertical.pack(side="right", fill="y")
        horizontal.pack(side="bottom", fill="x")
        self.editor.pack(fill="both", expand=True)
        self.editor.bind("<<Modified>>", self.source_edited)

        sidebar = ttk.Notebook(body)
        body.add(sidebar, minsize=280, width=350, stretch="never")
        catalog = tk.Frame(sidebar, bg=PANEL, padx=12, pady=10)
        sidebar.add(catalog, text="Instruments")
        library = tk.Frame(sidebar, bg=PANEL, padx=12, pady=10)
        sidebar.add(library, text="Saved music")
        self.build_library(library)
        self.catalog_count_var = tk.StringVar(value="INSTRUMENTS · awaiting catalog")
        tk.Label(catalog, textvariable=self.catalog_count_var, bg=PANEL, fg=GOLD,
                 anchor="w", font=("Consolas", 10, "bold")).pack(fill="x")
        self.catalog_search_var = tk.StringVar()
        search = tk.Entry(catalog, textvariable=self.catalog_search_var, bg=BACKGROUND,
                          fg=TEXT, insertbackground=CYAN, relief="flat")
        search.pack(fill="x", pady=(10, 8), ipady=5)
        self.catalog_search_var.trace_add("write", lambda *_args: self.refresh_catalog())
        browser = tk.Frame(catalog, bg=PANEL)
        browser.pack(fill="both", expand=True)
        self.catalog_tree = ttk.Treeview(browser, columns=("group",), show="tree headings",
                                        height=11, selectmode="browse", style="PyMusic.Treeview")
        self.catalog_tree.heading("#0", text="Instrument")
        self.catalog_tree.heading("group", text="Family")
        self.catalog_tree.column("#0", width=165, minwidth=100)
        self.catalog_tree.column("group", width=115, minwidth=70)
        browser_scroll = ttk.Scrollbar(browser, command=self.catalog_tree.yview)
        self.catalog_tree.configure(yscrollcommand=browser_scroll.set)
        browser_scroll.pack(side="right", fill="y")
        self.catalog_tree.pack(fill="both", expand=True)
        self.catalog_tree.bind("<<TreeviewSelect>>", lambda _event: self.show_instrument())
        self.catalog_detail_var = tk.StringVar(value="The complete playable catalog arrives from the music engine.")
        tk.Label(catalog, textvariable=self.catalog_detail_var, bg=PANEL, fg=TEXT,
                 justify="left", anchor="nw", wraplength=310, pady=10).pack(fill="x")
        self.snippet_var = tk.StringVar()
        tk.Label(catalog, textvariable=self.snippet_var, bg=BACKGROUND, fg=CYAN,
                 justify="left", anchor="nw", wraplength=310, padx=8, pady=8,
                 font=("Consolas", 9)).pack(fill="x")
        tk.Button(catalog, text="Insert instrument note", command=self.insert_instrument,
                  bg=PANEL, fg=GOLD, relief="flat", pady=8).pack(fill="x")
        tk.Label(catalog, text="Choose voices for the phrase. No quota or preset rotation.",
                 bg=PANEL, fg=MUTED, wraplength=310, justify="left").pack(fill="x", pady=5)
        self.status_var = tk.StringVar(value="Run sends a complete piece. Play resumes accepted audio.")
        tk.Label(self.root, textvariable=self.status_var, bg=BACKGROUND, fg=CYAN,
                 anchor="w", padx=18, pady=6, wraplength=1150).pack(fill="x")

    def build_library(self, panel: tk.Frame) -> None:
        self.library_count_var = tk.StringVar(value="SAVED MUSIC · 0 pieces")
        tk.Label(panel, textvariable=self.library_count_var, bg=PANEL, fg=GOLD,
                 anchor="w", font=("Consolas", 10, "bold")).pack(fill="x")
        tk.Label(panel, text="Complete verified pieces are saved here. Keep and Reject record your taste.",
                 bg=PANEL, fg=MUTED, wraplength=310, justify="left").pack(fill="x", pady=10)
        self.library_tree = ttk.Treeview(panel, columns=("duration", "verdict"), show="tree headings",
                                        selectmode="browse", style="PyMusic.Treeview")
        self.library_tree.heading("#0", text="Piece")
        self.library_tree.heading("duration", text="Time")
        self.library_tree.heading("verdict", text="Verdict")
        self.library_tree.column("#0", width=155, minwidth=90)
        self.library_tree.column("duration", width=50, stretch=False)
        self.library_tree.column("verdict", width=80, stretch=False)
        self.library_tree.pack(fill="both", expand=True)
        self.library_tree.bind("<<TreeviewSelect>>", lambda _event: self.show_library_selection())
        self.library_detail_var = tk.StringVar(value="No complete pieces saved yet.")
        tk.Label(panel, textvariable=self.library_detail_var, bg=PANEL, fg=TEXT,
                 wraplength=310, justify="left", anchor="nw", pady=10).pack(fill="x")
        self.library_play_button = tk.Button(panel, text="Play selected piece", command=self.play_library,
                                             bg=PANEL, fg=CYAN, relief="flat", pady=8, state="disabled")
        self.library_play_button.pack(fill="x")

    def update_library(self, entries: object, active_id: object) -> None:
        selected = self.library_tree.selection()
        self.library = library_entries(entries)
        self.active_library_id = active_id if valid_library_id(active_id) else None
        self.library_tree.delete(*self.library_tree.get_children())
        for entry in self.library:
            seconds = round(entry["duration_seconds"])
            title = ("▶ " if entry["id"] == self.active_library_id else "") + entry.get("display_title", entry["title"])
            self.library_tree.insert("", "end", iid=entry["id"], text=title,
                                     values=(f"{seconds // 60}:{seconds % 60:02d}", entry["verdict"]))
        rows = self.library_tree.get_children()
        self.library_count_var.set(f"SAVED MUSIC · {len(rows)} pieces")
        self.library_play_button.configure(state="normal" if rows else "disabled")
        if rows:
            target = (selected[0] if selected and selected[0] in rows else
                      self.active_library_id if self.active_library_id in rows else rows[0])
            self.library_tree.selection_set(target)
        self.show_library_selection()

    def show_library_selection(self) -> None:
        selected = self.library_tree.selection()
        entry = next((item for item in self.library if selected and item["id"] == selected[0]), None)
        if entry is None:
            self.library_detail_var.set("No complete pieces saved yet.")
            return
        active = "Currently selected for playback.\n" if entry["id"] == self.active_library_id else ""
        self.library_detail_var.set(
            f'{entry.get("display_title", entry["title"])}\n{library_saved_time(entry)}{active}'
            f'{entry["note_count"]} notes · {entry["duration_seconds"]:.1f} seconds\n'
            f'Listener verdict: {entry["verdict"]}.\nKeep and Reject apply to the piece playing, not this list selection.')

    def play_library(self) -> None:
        selected = self.library_tree.selection()
        if selected and any(entry["id"] == selected[0] for entry in self.library):
            self.send_action("save", code=self.code())
            self.send_action("library_play", id=selected[0])
            self.status_var.set("Loading saved audio; your source draft is retained.")

    def code(self) -> str:
        return self.editor.get("1.0", "end-1c")

    def source_edited(self, _event=None) -> None:
        self.document_var.set("Edited source · Save or Run" if self.code() != self.source else "Current source")
        self.editor.edit_modified(False)

    def reload_source(self) -> None:
        self.editor.delete("1.0", "end")
        self.editor.insert("1.0", self.source)
        self.editor.edit_reset()
        self.editor.edit_modified(False)
        self.document_var.set("Current source")

    def send_action(self, action: str, **fields: object) -> None:
        if self.closing:
            return
        try:
            self.emit(protocol_action(action, **fields))
        except (BrokenPipeError, OSError):
            self.close(notify=False)

    def submit_source(self, action: str) -> str:
        self.send_action(action, code=self.code())
        if not self.closing:
            self.status_var.set("Source sent for rendering" if action == "run" else "Source sent for saving")
        return "break"

    def control_action(self, action: str) -> None:
        # Save before switching: the runtime can invalidate this window's
        # session while it is shutting down, after receiving the switch action.
        if action in {"native", "compose"}:
            self.send_action("save", code=self.code())
        self.send_action(action)

    def refresh_catalog(self) -> None:
        selected = self.catalog_tree.selection()
        self.catalog_tree.delete(*self.catalog_tree.get_children())
        query = self.catalog_search_var.get().strip().casefold()
        for info in self.instrument_catalog:
            if query and query not in " ".join(str(info.get(key, "")) for key in
                                               ("id", "name", "group", "role", "description")).casefold():
                continue
            self.catalog_tree.insert("", "end", iid=info["id"], text=info.get("name", info["id"]),
                                     values=(info.get("group", ""),))
        rows = self.catalog_tree.get_children()
        self.catalog_count_var.set(f"INSTRUMENTS · {len(rows)} / {len(self.instrument_catalog)}")
        if rows:
            self.catalog_tree.selection_set(selected[0] if selected and selected[0] in rows else rows[0])
            self.show_instrument()
        else:
            self.catalog_detail_var.set("No voices match this search." if self.instrument_catalog
                                        else "The complete playable catalog arrives from the music engine.")
            self.snippet_var.set("")

    def show_instrument(self) -> None:
        selected = self.catalog_tree.selection()
        info = next((item for item in self.instrument_catalog if selected and item["id"] == selected[0]), None)
        if info is None:
            return
        low, high = info["range"]
        line = "One melodic note at a time. " if info.get("monophonic") else ""
        self.catalog_detail_var.set(
            f'{info.get("name", info["id"])} · {info.get("role", "")}\n'
            f'ID: {info["id"]}\nWorking MIDI range: {low}–{high}\n'
            f'Attack spacing: {info.get("attack_spacing", 0)} beats\n'
            f'{line}{info.get("description", "")}\nNote duration below is in seconds.')
        self.snippet_var.set(instrument_snippet(info))

    def insert_instrument(self) -> None:
        if self.snippet_var.get():
            self.editor.insert("insert", self.snippet_var.get())
            self.editor.focus_set()

    def read_commands(self, stream: TextIO | None = None) -> None:
        try:
            for line in stream or sys.stdin:
                try:
                    value = json.loads(line)
                except (ValueError, TypeError):
                    continue
                if isinstance(value, dict):
                    self.commands.put(value)
        finally:
            self.commands.put({"action": "shutdown"})

    def apply_command(self, command: dict) -> None:
        action = command.get("action")
        if action == "source":
            incoming = command.get("code")
            if isinstance(incoming, str):
                edited = self.code() != self.source
                self.source = incoming
                if not edited or self.code() == incoming:
                    self.reload_source()
                else:
                    self.status_var.set("New source available. Your edits are retained; Reload replaces them.")
            if "instrument_catalog" in command:
                self.instrument_catalog = catalog_entries(command["instrument_catalog"])
                self.refresh_catalog()
            if "library" in command:
                self.update_library(command["library"], command.get("active_library_id"))
            if type(command.get("alternating")) is bool:
                self.alternating = command["alternating"]
                self.alternate_var.set("Alternate: on" if self.alternating else "Alternate: off")
            self.set_transport(command.get("transport"))
        elif action == "transport":
            self.set_transport(command.get("state"))
        elif action == "library":
            self.update_library(command.get("entries"), command.get("active_id"))
        elif action == "status" and isinstance(command.get("message"), str):
            self.status_var.set(command["message"])
        elif action == "alternating" and type(command.get("enabled")) is bool:
            self.alternating = command["enabled"]
            self.alternate_var.set("Alternate: on" if self.alternating else "Alternate: off")
        elif action in {"shutdown", "close"}:
            self.close()

    def set_transport(self, state: object) -> None:
        if isinstance(state, str) and state in TRANSPORTS:
            self.transport = state
            self.transport_var.set(state.upper())

    def tick(self) -> None:
        self.after_id = None
        while not self.closing:
            try:
                self.apply_command(self.commands.get_nowait())
            except queue.Empty:
                break
        if not self.closing:
            self.after_id = self.root.after(40, self.tick)

    def close(self, notify: bool = True) -> None:
        if self.closing:
            return
        if notify:
            self.send_action("save", code=self.code())
            self.send_action("close")
        self.closing = True
        if self.after_id is not None:
            self.root.after_cancel(self.after_id)
            self.after_id = None
        try:
            self.root.destroy()
        except tk.TclError:
            pass


def main() -> int:
    from daw import claim_single_window
    claim_single_window("python-music")
    for stream in (sys.stdin, sys.stdout):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8")
    root = tk.Tk()
    PythonMusicEditor(root)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

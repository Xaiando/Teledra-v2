"""Organist Studio for Teledra's verified native Court Synth.

This process is deliberately a control and visualization surface only. Rust
owns score validation, synthesis, acoustic verification, persistence, and the
single audio stream.
"""

from __future__ import annotations

import json
import math
import queue
import sys
import threading
import time
import tkinter as tk
from tkinter import ttk
from typing import Any


BACKGROUND = "#060910"
PANEL = "#0e1622"
PANEL_ALT = "#121d2b"
GRID = "#26374b"
GOLD = "#e6bd6a"
CYAN = "#6ee7df"
MUTED = "#8291a6"
TEXT = "#eef3f8"
GREEN = "#61d6a9"
TRACK_COLORS = (
    "#61d6a9",
    "#71b7ff",
    "#df85ff",
    "#f4a261",
    "#e76f8a",
    "#a6d96a",
    "#ffd166",
    "#8ecae6",
)
CADENCES = {
    "Held · 2 min": 120,
    "Long · 5 min": 300,
    "Epic · 10 min": 600,
}
ALLOWED_ACTIONS = {
    "play",
    "pause",
    "stop",
    "evolve",
    "new_movement",
    "python_music",
    "toggle_alternation",
    "keep",
    "reject",
    "set_cadence",
    "set_track_gain",
    "set_track_pan",
}


def protocol_action(action: str, **fields: object) -> dict[str, object]:
    """Construct one allow-listed Rust command and reject loose values."""
    if action not in ALLOWED_ACTIONS:
        raise ValueError(f"unsupported DAW action: {action}")
    payload: dict[str, object] = {"action": action, **fields}
    if action == "set_cadence":
        seconds = int(payload.get("seconds", 0))
        if not 60 <= seconds <= 900:
            raise ValueError("cadence must be 60..900 seconds")
        payload = {"action": action, "seconds": seconds}
    elif action in {"set_track_gain", "set_track_pan"}:
        track = int(payload.get("track", -1))
        value_name = "gain" if action.endswith("gain") else "pan"
        value = float(payload.get(value_name, math.nan))
        low, high = ((0.0, 1.0) if value_name == "gain" else (-1.0, 1.0))
        if not 0 <= track < 8 or not math.isfinite(value) or not low <= value <= high:
            raise ValueError("invalid bounded mixer action")
        payload = {"action": action, "track": track, value_name: round(value, 3)}
    elif fields:
        raise ValueError(f"{action} does not accept fields")
    return payload


def score_duration(score: dict[str, object]) -> float:
    bpm = max(40, int(score.get("bpm", 92) or 92))
    bars = max(1, int(score.get("bars", 1) or 1))
    return bars * 4.0 * 60.0 / bpm


def score_grid(score: dict[str, object]) -> int:
    plan = score.get("composition") or {}
    value = plan.get("steps_per_bar", 16) if isinstance(plan, dict) else 16
    return value if value in (4, 8, 16) else 16


def sequencer_note(steps: list, column: int, steps_per_bar: int = 16):
    """Written onsets on the score's common grid, including complete phrases."""
    if not steps:
        return None
    divisions = min(steps_per_bar, len(steps))
    step = round(column * divisions / steps_per_bar)
    if round(step * steps_per_bar / divisions) != column:
        return None
    return steps[step % len(steps)]


def expressed_steps(track: dict[str, object]) -> list:
    """A zero-velocity note is displayed as a rest; retain the authored score."""
    steps = track.get("steps", [])
    if not isinstance(steps, list) or not steps:
        return [None]
    velocities = track.get("velocities")
    return [None if isinstance(velocities, list) and index < len(velocities)
            and velocities[index] == 0 else degree for index, degree in enumerate(steps)]


def sequencer_column(score: dict[str, object], position_seconds: float, columns: int,
                     form_timeline: list[tuple[str, int, int]]) -> int:
    """Follow the renderer's pattern restart at each explicit form boundary."""
    bpm = max(40, int(score.get("bpm", 92)))
    grid = score_grid(score)
    step = int(max(0.0, position_seconds) * bpm / 240.0 * grid)
    bar = step // grid
    plan = score.get("composition") or {}
    if isinstance(plan, dict) and plan.get("form", "arc") != "arc":
        start = next((start for _, start, end in form_timeline if start <= bar < end), 0)
        step -= start * grid
    return step % max(1, columns)


def movement_name(evolution: int) -> str:
    if evolution <= 0:
        return "STATEMENT"
    return ("MODULATION", "PULSE", "MOTIF", "COUNTERPOINT", "TEXTURE", "HARMONY")[
        evolution % 6
    ]


def chord_name(score: dict[str, object], chord: dict[str, object]) -> str:
    notes = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")
    scales = {
        "major": (0, 2, 4, 5, 7, 9, 11),
        "minor": (0, 2, 3, 5, 7, 8, 10),
        "dorian": (0, 2, 3, 5, 7, 9, 10),
        "mixolydian": (0, 2, 4, 5, 7, 9, 10),
        "lydian": (0, 2, 4, 6, 7, 9, 11),
        "phrygian": (0, 1, 3, 5, 7, 8, 10),
        "harmonic_minor": (0, 2, 3, 5, 7, 8, 11),
    }
    tonic = int(score.get("tonic", 0) or 0) % 12
    mode = str(score.get("mode", "minor")).lower()
    degree = int(chord.get("root_degree", 0) or 0) % 7
    quality = str(chord.get("quality", "major"))
    suffix = {
        "major": "",
        "minor": "m",
        "diminished": "°",
        "augmented": "+",
        "major_seventh": "maj7",
        "minor_seventh": "m7",
        "dominant_seventh": "7",
        "half_diminished": "ø7",
        "suspended2": "sus2",
        "suspended4": "sus4",
    }.get(quality, "")
    return notes[(tonic + scales.get(mode, scales["minor"])[degree]) % 12] + suffix


def chord_cycle_state(score: dict[str, object], position_seconds: float,
                      harmonic_timeline: list[int] | None = None) -> tuple[int, int, int, int]:
    """Current authored harmony, using the same chord duration as the arranger."""
    plan = score.get("composition") or {}
    chord_bars = max(1, int(plan.get("chord_bars", 1))) if isinstance(plan, dict) else 1
    progression = score.get("progression") or []
    length = len(progression) if isinstance(progression, list) else 0
    bars = max(1, int(score.get("bars", 1)))
    bpm = max(40, int(score.get("bpm", 92)))
    bar = int(max(0.0, position_seconds) * bpm / 240.0) % bars
    current = ((harmonic_timeline[bar] if harmonic_timeline and len(harmonic_timeline) == bars
                else bar // chord_bars) % max(1, length))
    return current, bar, chord_bars, length * chord_bars


class OrganistStudio:
    def __init__(self, root: tk.Tk) -> None:
        self.root = root
        self.commands: queue.Queue[dict[str, object]] = queue.Queue()
        self.score: dict[str, object] = {}
        self.analysis: dict[str, object] = {}
        self.theory: dict[str, object] = {}
        self.form_timeline: list[tuple[str, int, int]] = []
        self.harmonic_timeline: list[int] = []
        self.instrument_catalog: list[dict[str, Any]] = []
        self.catalog_window: tk.Toplevel | None = None
        self.transport = "stopped"
        self.position_seconds = 0.0
        self.last_clock = time.monotonic()
        self.cadence_seconds = 300
        self.next_change_at = self.last_clock + self.cadence_seconds
        self.muted_gains: dict[int, float] = {}
        self.track_controls: list[dict[str, Any]] = []
        self.closing = False

        root.title("Organist Studio // Court Synth")
        root.geometry("1320x820+18+88")
        root.minsize(980, 640)
        root.configure(bg=BACKGROUND)
        self.configure_styles()
        self.build_header()
        self.build_transport()
        self.build_workspace()
        self.build_footer()
        self.bind_shortcuts()
        root.protocol("WM_DELETE_WINDOW", self.close)

        threading.Thread(target=self.read_commands, daemon=True).start()
        print(json.dumps({"event": "ready", "protocol": 2}), flush=True)
        root.after(40, self.tick)

    def configure_styles(self) -> None:
        style = ttk.Style(self.root)
        style.theme_use("clam")
        style.configure(
            "Daw.TButton",
            background=PANEL_ALT,
            foreground=TEXT,
            bordercolor=GRID,
            padding=(11, 7),
            font=("Segoe UI", 9, "bold"),
        )
        style.map("Daw.TButton", background=[("active", "#1d3045")])
        style.configure(
            "Accent.TButton",
            background="#1f5a61",
            foreground="#f4ffff",
            bordercolor=CYAN,
            padding=(13, 8),
            font=("Segoe UI", 9, "bold"),
        )
        style.map("Accent.TButton", background=[("active", "#27727b")])
        style.configure(
            "Gold.TButton",
            background="#4b3c1d",
            foreground="#fff3ce",
            bordercolor=GOLD,
            padding=(13, 8),
            font=("Segoe UI", 9, "bold"),
        )
        style.map("Gold.TButton", background=[("active", "#6a5426")])
        style.configure("Daw.Horizontal.TScale", background=PANEL, troughcolor="#1b2939")
        style.configure("Catalog.Treeview", background=PANEL_ALT, fieldbackground=PANEL_ALT,
                        foreground=TEXT, rowheight=27, font=("Segoe UI", 10))
        style.configure("Catalog.Treeview.Heading", font=("Segoe UI", 9, "bold"))
        style.map("Catalog.Treeview", background=[("selected", "#24505a")],
                  foreground=[("selected", "#ffffff")])
        style.configure(
            "Daw.TCombobox",
            fieldbackground=PANEL_ALT,
            background=PANEL_ALT,
            foreground=TEXT,
            arrowcolor=GOLD,
        )

    def build_header(self) -> None:
        header = tk.Frame(self.root, bg=BACKGROUND, padx=20, pady=13)
        header.pack(fill="x")
        brand = tk.Frame(header, bg=BACKGROUND)
        brand.pack(side="left")
        tk.Label(
            brand,
            text="ORGANIST STUDIO",
            bg=BACKGROUND,
            fg=GOLD,
            font=("Consolas", 20, "bold"),
        ).pack(anchor="w")
        tk.Label(
            brand,
            text="Themes, harmony, and musical discoveries",
            bg=BACKGROUND,
            fg=MUTED,
            font=("Consolas", 9),
        ).pack(anchor="w")

        self.title_var = tk.StringVar(value="Waiting for a Court Score")
        self.movement_var = tk.StringVar(value="STATEMENT")
        title_box = tk.Frame(header, bg=BACKGROUND)
        title_box.pack(side="left", padx=(34, 0), fill="x", expand=True)
        tk.Label(
            title_box,
            textvariable=self.title_var,
            bg=BACKGROUND,
            fg=TEXT,
            font=("Segoe UI", 14, "bold"),
            anchor="w",
        ).pack(fill="x")
        tk.Label(
            title_box,
            textvariable=self.movement_var,
            bg=BACKGROUND,
            fg=CYAN,
            font=("Consolas", 10, "bold"),
            anchor="w",
        ).pack(fill="x")

        self.transport_var = tk.StringVar(value="STOPPED")
        tk.Label(
            header,
            textvariable=self.transport_var,
            bg=PANEL_ALT,
            fg=GOLD,
            padx=15,
            pady=8,
            font=("Consolas", 11, "bold"),
        ).pack(side="right")

    def build_transport(self) -> None:
        strip = tk.Frame(self.root, bg=PANEL, padx=18, pady=10)
        strip.pack(fill="x", padx=18)
        for label, action in (("▶ Play", "play"), ("Ⅱ Pause", "pause"), ("■ Stop", "stop")):
            ttk.Button(
                strip,
                text=label,
                style="Daw.TButton",
                command=lambda selected=action: self.send_action(selected),
            ).pack(side="left", padx=(0, 6))
        ttk.Button(
            strip,
            text="EVOLVE NOW",
            style="Accent.TButton",
            command=lambda: self.send_action("evolve"),
        ).pack(side="left", padx=(12, 6))
        ttk.Button(
            strip,
            text="NEW MOVEMENT",
            style="Gold.TButton",
            command=lambda: self.send_action("new_movement"),
        ).pack(side="left", padx=6)
        ttk.Button(strip, text="PyMusic", style="Daw.TButton",
                   command=lambda: self.send_action("python_music")).pack(side="left", padx=4)
        self.alternate_var = tk.StringVar(value="Alternate: off")
        ttk.Button(strip, textvariable=self.alternate_var, style="Daw.TButton",
                   command=lambda: self.send_action("toggle_alternation")).pack(side="left", padx=4)

        self.countdown_var = tk.StringVar(value="NEXT DEVELOPMENT --")
        tk.Label(
            strip,
            textvariable=self.countdown_var,
            bg=PANEL,
            fg=CYAN,
            font=("Consolas", 10, "bold"),
        ).pack(side="right", padx=(12, 0))
        self.cadence_var = tk.StringVar(value="Flowing · 36s")
        cadence = ttk.Combobox(
            strip,
            textvariable=self.cadence_var,
            values=list(CADENCES),
            state="readonly",
            width=17,
            style="Daw.TCombobox",
        )
        cadence.pack(side="right")
        cadence.bind("<<ComboboxSelected>>", self.change_cadence)
        tk.Label(strip, text="CHANGE RATE", bg=PANEL, fg=MUTED, font=("Consolas", 8)).pack(
            side="right", padx=(0, 8)
        )

    def build_workspace(self) -> None:
        body = tk.PanedWindow(
            self.root,
            orient="horizontal",
            bg=BACKGROUND,
            sashwidth=5,
            sashrelief="flat",
            bd=0,
        )
        body.pack(fill="both", expand=True, padx=18, pady=12)

        mixer_panel = tk.Frame(body, bg=PANEL, width=350)
        body.add(mixer_panel, minsize=300, width=350)
        tk.Label(
            mixer_panel,
            text="MIXER · SHAPE THE ENSEMBLE",
            bg=PANEL,
            fg=GOLD,
            font=("Consolas", 10, "bold"),
            anchor="w",
            padx=12,
            pady=10,
        ).pack(fill="x")
        mixer_shell = tk.Frame(mixer_panel, bg=PANEL)
        mixer_shell.pack(fill="both", expand=True)
        self.mixer_canvas = tk.Canvas(mixer_shell, bg=PANEL, highlightthickness=0, width=340)
        mixer_scroll = ttk.Scrollbar(mixer_shell, orient="vertical", command=self.mixer_canvas.yview)
        self.mixer_canvas.configure(yscrollcommand=mixer_scroll.set)
        mixer_scroll.pack(side="right", fill="y")
        self.mixer_canvas.pack(side="left", fill="both", expand=True)
        self.mixer_rows = tk.Frame(self.mixer_canvas, bg=PANEL)
        self.mixer_window = self.mixer_canvas.create_window((0, 0), window=self.mixer_rows, anchor="nw")
        self.mixer_rows.bind(
            "<Configure>",
            lambda _event: self.mixer_canvas.configure(scrollregion=self.mixer_canvas.bbox("all")),
        )
        self.mixer_canvas.bind(
            "<Configure>",
            lambda event: self.mixer_canvas.itemconfigure(self.mixer_window, width=event.width),
        )

        right = tk.Frame(body, bg=BACKGROUND)
        body.add(right, minsize=600)
        top = tk.Frame(right, bg=BACKGROUND)
        top.pack(fill="x", pady=(0, 10))
        self.info_var = tk.StringVar(value="Waiting for the next composition")
        tk.Label(
            top,
            textvariable=self.info_var,
            bg=PANEL,
            fg=CYAN,
            font=("Consolas", 10),
            anchor="w",
            padx=12,
            pady=9,
        ).pack(side="left", fill="x", expand=True)
        self.quality_var = tk.StringVar(value="SCORE CHECKS --")
        tk.Label(
            top,
            textvariable=self.quality_var,
            bg="#13251f",
            fg=GREEN,
            font=("Consolas", 10, "bold"),
            padx=12,
            pady=9,
        ).pack(side="right", padx=(8, 0))

        self.timeline = tk.Canvas(right, height=118, bg=PANEL, highlightthickness=1, highlightbackground=GRID)
        self.timeline.pack(fill="x", pady=(0, 10))
        self.timeline.bind("<Configure>", lambda _event: self.draw_timeline())
        self.canvas = tk.Canvas(right, bg="#090e17", highlightthickness=1, highlightbackground=GRID)
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<Configure>", lambda _event: self.draw_sequencer())

        theory_panel = tk.Frame(right, bg=PANEL, padx=12, pady=8)
        theory_panel.pack(fill="x", pady=(10, 0))
        self.theory_var = tk.StringVar(value="The Organist is studying the next musical idea")
        self.experiment_var = tk.StringVar(value="")
        theory_label = tk.Label(theory_panel, textvariable=self.theory_var, bg=PANEL,
                                fg=CYAN, font=("Segoe UI", 9), anchor="w", justify="left",
                                wraplength=550)
        theory_label.pack(fill="x")
        experiment_label = tk.Label(theory_panel, textvariable=self.experiment_var, bg=PANEL,
                                    fg=TEXT, font=("Segoe UI", 9), anchor="w", justify="left",
                                    wraplength=550)
        experiment_label.pack(fill="x", pady=(5, 0))
        theory_panel.bind("<Configure>", lambda event: (
            theory_label.configure(wraplength=max(250, event.width - 24)),
            experiment_label.configure(wraplength=max(250, event.width - 24))))

        self.craft_var = tk.StringVar(value="The Organist is preparing the craft analysis")
        tk.Label(
            right,
            textvariable=self.craft_var,
            bg=PANEL,
            fg=MUTED,
            font=("Consolas", 9),
            anchor="w",
            padx=10,
            pady=7,
        ).pack(fill="x", pady=(10, 0))

    def build_footer(self) -> None:
        footer = tk.Frame(self.root, bg=BACKGROUND, padx=18, pady=8)
        footer.pack(fill="x")
        ttk.Button(
            footer,
            text="KEEP",
            style="Daw.TButton",
            command=lambda: self.send_action("keep"),
        ).pack(side="left")
        ttk.Button(
            footer,
            text="REJECT",
            style="Daw.TButton",
            command=lambda: self.send_action("reject"),
        ).pack(side="left", padx=6)
        self.catalog_button_var = tk.StringVar(value="INSTRUMENTS")
        ttk.Button(footer, textvariable=self.catalog_button_var, style="Daw.TButton",
                   command=self.open_catalog).pack(side="left", padx=(8, 0))
        self.status_var = tk.StringVar(
            value="Keep an idea you like, or ask for a new movement"
        )
        tk.Label(
            footer,
            textvariable=self.status_var,
            bg=BACKGROUND,
            fg=MUTED,
            font=("Consolas", 9),
            anchor="e",
        ).pack(side="right", fill="x", expand=True)

    def bind_shortcuts(self) -> None:
        self.root.bind(
            "<space>",
            lambda _event: self.send_action("pause" if self.transport in {"playing", "muted"} else "play"),
        )
        self.root.bind("<Control-e>", lambda _event: self.send_action("evolve"))
        self.root.bind("<Control-n>", lambda _event: self.send_action("new_movement"))

    def open_catalog(self) -> None:
        if self.catalog_window is not None and self.catalog_window.winfo_exists():
            self.catalog_window.lift()
            return
        window = tk.Toplevel(self.root)
        self.catalog_window = window
        window.title("Organist Studio // Instrument Library")
        window.geometry("890x660")
        window.minsize(680, 460)
        window.configure(bg=BACKGROUND)
        tk.Label(window, text="INSTRUMENT LIBRARY", bg=BACKGROUND, fg=GOLD,
                 font=("Consolas", 18, "bold"), anchor="w").pack(fill="x", padx=18, pady=(16, 4))
        tk.Label(window, text="Find a voice and ask the Organist to compose with it.",
                 bg=BACKGROUND, fg=MUTED, font=("Segoe UI", 10), anchor="w").pack(fill="x", padx=18)
        search_bar = tk.Frame(window, bg=BACKGROUND)
        search_bar.pack(fill="x", padx=18, pady=12)
        tk.Label(search_bar, text="SEARCH", bg=BACKGROUND, fg=CYAN,
                 font=("Consolas", 10, "bold")).pack(side="left", padx=(0, 10))
        self.catalog_search_var = tk.StringVar()
        search = tk.Entry(search_bar, textvariable=self.catalog_search_var, bg=PANEL_ALT,
                          fg=TEXT, insertbackground=TEXT, relief="flat", font=("Segoe UI", 11))
        search.pack(side="left", fill="x", expand=True, ipady=5)
        self.catalog_count_var = tk.StringVar()
        tk.Label(search_bar, textvariable=self.catalog_count_var, bg=BACKGROUND, fg=MUTED,
                 font=("Consolas", 9)).pack(side="right", padx=(12, 0))
        table = tk.Frame(window, bg=PANEL)
        table.pack(fill="both", expand=True, padx=18)
        self.catalog_tree = ttk.Treeview(table, columns=("name", "group", "role", "range"),
                                        show="headings", selectmode="browse", style="Catalog.Treeview")
        for key, label, width in (("name", "Instrument", 240), ("group", "Family", 215),
                                  ("role", "Role", 110), ("range", "MIDI range", 110)):
            self.catalog_tree.heading(key, text=label)
            self.catalog_tree.column(key, width=width, minwidth=85, stretch=True)
        scroll = ttk.Scrollbar(table, orient="vertical", command=self.catalog_tree.yview)
        self.catalog_tree.configure(yscrollcommand=scroll.set)
        scroll.pack(side="right", fill="y")
        self.catalog_tree.pack(fill="both", expand=True)
        self.catalog_detail_var = tk.StringVar(value="Select an instrument to see its playing advice.")
        detail = tk.Label(window, textvariable=self.catalog_detail_var, bg=PANEL, fg=TEXT,
                          font=("Segoe UI", 10), anchor="nw", justify="left", wraplength=800,
                          padx=12, pady=12, height=5)
        detail.pack(fill="x", padx=18, pady=14)
        detail.bind("<Configure>", lambda event: detail.configure(wraplength=max(300, event.width - 24)))
        self.catalog_search_var.trace_add("write", lambda *_args: self.refresh_catalog())
        self.catalog_tree.bind("<<TreeviewSelect>>", lambda _event: self.show_catalog_detail())
        # Keep transport hotkeys in the main Studio: typing a space here searches.
        self.refresh_catalog()
        search.focus_set()

    def refresh_catalog(self) -> None:
        self.catalog_button_var.set(f"INSTRUMENTS · {len(self.instrument_catalog)}"
                                    if self.instrument_catalog else "INSTRUMENTS")
        if self.catalog_window is None or not self.catalog_window.winfo_exists():
            return
        query = self.catalog_search_var.get().strip().casefold()
        selected = self.catalog_tree.selection()
        for item in self.catalog_tree.get_children():
            self.catalog_tree.delete(item)
        for info in self.instrument_catalog:
            search_text = " ".join(str(info.get(key, "")) for key in
                                   ("id", "name", "group", "role", "description")).casefold()
            if query and query not in search_text:
                continue
            low, high = info["range"]
            self.catalog_tree.insert("", "end", iid=info["id"], values=(info["name"],
                info["group"], str(info["role"]).title(), f"{low}–{high}"))
        rows = self.catalog_tree.get_children()
        self.catalog_count_var.set(f"{len(rows)} / {len(self.instrument_catalog)} voices")
        if rows:
            self.catalog_tree.selection_set(selected[0] if selected and selected[0] in rows else rows[0])
            self.show_catalog_detail()
        else:
            self.catalog_detail_var.set("No voices match this search." if self.instrument_catalog
                                        else "The instrument catalog arrives with the next accepted score.")

    def show_catalog_detail(self) -> None:
        selected = self.catalog_tree.selection()
        if not selected:
            return
        info = next((entry for entry in self.instrument_catalog if entry["id"] == selected[0]), None)
        if info is None:
            return
        low, high = info["range"]
        playing = "One melodic note at a time. " if info.get("monophonic") else ""
        register = ("Unpitched rhythm: degree 0 is a hit; null is silence."
                    if info.get("role") == "percussion" else
                    f"{playing}Sounding MIDI range {low}–{high} (C4 = 60). "
                    "Choose a register that keeps the whole phrase in range.")
        self.catalog_detail_var.set(
            f"{info['name']} · {info['id']}\n{info.get('description', '')}\n"
            f"{register}")

    def read_commands(self) -> None:
        for raw in sys.stdin:
            try:
                value = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict):
                self.commands.put(value)

    def send_action(self, action: str, **fields: object) -> None:
        try:
            payload = protocol_action(action, **fields)
        except (TypeError, ValueError) as error:
            self.status_var.set(str(error))
            return
        print(json.dumps(payload, separators=(",", ":")), flush=True)
        labels = {
            "evolve": "Native movement requested",
            "new_movement": "The Organist is composing a genuinely new movement",
            "keep": "Keeper verdict recorded",
            "reject": "Rejection verdict recorded",
        }
        if action in labels:
            self.status_var.set(labels[action])

    def apply_command(self, command: dict[str, object]) -> None:
        if command.get("action") == "alternating" and type(command.get("enabled")) is bool:
            self.alternate_var.set("Alternate: on" if command["enabled"] else "Alternate: off")
            return
        action = command.get("action")
        if action == "shutdown":
            self.close()
        elif action == "score" and isinstance(command.get("score"), dict):
            self.advance_clock()
            old_names = [str(track.get("name", "")) for track in self.tracks()]
            old_piece = str(self.score.get("title", "")).split(" // ")[0]
            self.score = command["score"]  # type: ignore[assignment]
            new_names = [str(track.get("name", "")) for track in self.tracks()]
            new_piece = str(self.score.get("title", "")).split(" // ")[0]
            if old_names and (old_piece != new_piece or len(old_names) != len(new_names)):
                self.muted_gains.clear()
            analysis = command.get("analysis", {})
            self.analysis = analysis if isinstance(analysis, dict) else {}
            theory = command.get("theory", {})
            self.theory = theory if isinstance(theory, dict) else {}
            catalog = command.get("instrument_catalog")
            if isinstance(catalog, list):
                seen = set()
                self.instrument_catalog = []
                for entry in catalog[:256]:
                    if not isinstance(entry, dict) or not all(isinstance(entry.get(key), str)
                            for key in ("id", "name", "group", "role")):
                        continue
                    bounds = entry.get("range")
                    if not entry["id"] or entry["id"] in seen or not isinstance(bounds, list) \
                            or len(bounds) != 2 or not all(isinstance(value, int) for value in bounds) \
                            or not 0 <= bounds[0] <= bounds[1] <= 127:
                        continue
                    seen.add(entry["id"])
                    self.instrument_catalog.append(entry)
            self.refresh_catalog()
            timeline = command.get("form_timeline", [])
            bars = max(1, int(self.score.get("bars", 1)))
            self.form_timeline = [
                (str(item[0])[:24], int(item[1]), int(item[2]))
                for item in (timeline if isinstance(timeline, list) else [])[:64]
                if isinstance(item, (list, tuple)) and len(item) == 3
                and isinstance(item[1], int) and isinstance(item[2], int)
                and 0 <= item[1] < item[2] <= bars
            ]
            harmony_timeline = command.get("harmonic_timeline", [])
            self.harmonic_timeline = (list(harmony_timeline)
                if isinstance(harmony_timeline, list) and len(harmony_timeline) == bars
                and all(isinstance(index, int) and 0 <= index < 16 for index in harmony_timeline)
                else [])
            self.transport = str(command.get("transport", "playing"))
            position = command.get("position_seconds", 0.0)
            self.position_seconds = (float(position) % score_duration(self.score)
                if isinstance(position, (int, float)) and math.isfinite(position) and position >= 0
                else 0.0)
            self.cadence_seconds = max(
                60, min(900, int(command.get("cadence_seconds", 300) or 300))
            )
            self.last_clock = time.monotonic()
            self.next_change_at = self.last_clock + self.cadence_seconds
            self.sync_cadence_label()
            self.refresh_labels()
            self.rebuild_mixer()
            self.draw_all()
            self.status_var.set("Movement ready")
        elif action == "transport":
            self.advance_clock()
            previous = self.transport
            self.transport = str(command.get("state", "stopped"))
            if self.transport == "stopped":
                self.position_seconds = 0.0
            elif self.transport in {"playing", "muted"} and previous not in {"playing", "muted"}:
                self.next_change_at = time.monotonic() + self.cadence_seconds
            self.last_clock = time.monotonic()
            self.refresh_labels()
        elif action == "cadence":
            self.cadence_seconds = max(60, min(900, int(command.get("seconds", 300) or 300)))
            self.next_change_at = time.monotonic() + self.cadence_seconds
            self.sync_cadence_label()
        elif action == "status":
            self.status_var.set(str(command.get("message", ""))[:240])

    def advance_clock(self) -> None:
        now = time.monotonic()
        if self.transport in {"playing", "muted"} and self.score:
            self.position_seconds = (
                self.position_seconds + now - self.last_clock
            ) % score_duration(self.score)
        self.last_clock = now

    def sync_cadence_label(self) -> None:
        label = next((label for label, seconds in CADENCES.items() if seconds == self.cadence_seconds),
                     f"Auto · {self.cadence_seconds}s")
        self.cadence_var.set(label)

    def change_cadence(self, _event: object = None) -> None:
        seconds = CADENCES.get(self.cadence_var.get(), 300)
        self.cadence_seconds = seconds
        self.next_change_at = time.monotonic() + seconds
        self.send_action("set_cadence", seconds=seconds)

    def refresh_labels(self) -> None:
        title = str(self.score.get("title", "Court Score"))
        bpm = int(self.score.get("bpm", 0) or 0)
        bars = int(self.score.get("bars", 0) or 0)
        tonic = int(self.score.get("tonic", 0) or 0)
        mode = str(self.score.get("mode", "unknown")).replace("_", " ").upper()
        evolution = int(self.score.get("evolution", 0) or 0)
        notes = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")
        self.title_var.set(title)
        movement = str(self.analysis.get("movement", movement_name(evolution))).replace(
            "_", " "
        ).upper()
        self.movement_var.set(f"MOVEMENT {evolution:03d} · {movement}")
        self.info_var.set(
            f"{bpm} BPM   ·   {bars} BARS   ·   {notes[tonic % 12]} {mode}   ·   "
            f"{len(self.tracks())} TRACKS   ·   {score_duration(self.score):.1f}s"
        )
        quality = int(self.analysis.get("quality", 0) or 0)
        grade = str(self.analysis.get("grade", "UNASSESSED")).upper()
        self.quality_var.set(f"SCORE CHECKS {quality:03d}% · {grade}")
        checks = (
            ("pulse", "PULSE"),
            ("foundation", "FOUNDATION"),
            ("harmony", "HARMONY"),
            ("melody", "MELODY"),
            ("cadence", "CADENCE"),
            ("breathing_room", "SPACE"),
            ("palette", "PALETTE"),
            ("form", "FORM"),
            ("articulation", "ARTICULATION"),
            ("separation", "SEPARATION"),
            ("register_fit", "REGISTER"),
            ("independent_parts", "PARTS"),
        )
        summary = "  ".join(
            f"{'✓' if self.analysis.get(key) else '·'} {label}" for key, label in checks
        )
        guidance = self.analysis.get("guidance", [])
        if isinstance(guidance, list) and guidance:
            summary += "   NEXT: " + str(guidance[0]).upper()
        self.craft_var.set(summary)
        harmony = self.theory.get("progression", [])
        romans = " → ".join(str(chord.get("roman", "")) for chord in harmony
                           if isinstance(chord, dict)) if isinstance(harmony, list) else ""
        motif = self.theory.get("motif") or {}
        motif_text = (f"{motif.get('track', 'Theme')}: {motif.get('contour', 'developing')}"
                      if isinstance(motif, dict) and motif else "")
        key = str(self.theory.get("key", f"{notes[tonic % 12]} {mode.lower()}"))
        self.theory_var.set("  ·  ".join(part for part in (key, romans, motif_text) if part))
        experiment = str(self.theory.get("next_experiment", ""))
        self.experiment_var.set(f"TRY NEXT  {experiment}" if experiment else
                                "Listen for a motif, an answering phrase, and a recognisable return.")
        self.transport_var.set(self.transport.upper())

    def tracks(self) -> list[dict[str, object]]:
        tracks = self.score.get("tracks", [])
        return (
            [track for track in tracks if isinstance(track, dict)][:8]
            if isinstance(tracks, list)
            else []
        )

    def rebuild_mixer(self) -> None:
        for child in self.mixer_rows.winfo_children():
            child.destroy()
        self.track_controls.clear()
        for index, track in enumerate(self.tracks()):
            color = TRACK_COLORS[index % len(TRACK_COLORS)]
            row = tk.Frame(self.mixer_rows, bg=PANEL_ALT, padx=9, pady=7)
            row.pack(fill="x", padx=7, pady=(0, 6))
            tk.Frame(row, bg=color, width=5).pack(side="left", fill="y", padx=(0, 8))
            content = tk.Frame(row, bg=PANEL_ALT)
            content.pack(side="left", fill="both", expand=True)
            name = str(track.get("name", f"Track {index + 1}"))[:27]
            instrument = str(track.get("instrument") or track.get("waveform", "voice")).replace(
                "_", " "
            ).upper()
            head = tk.Frame(content, bg=PANEL_ALT)
            head.pack(fill="x")
            tk.Label(
                head, text=name, bg=PANEL_ALT, fg=TEXT, font=("Segoe UI", 9, "bold")
            ).pack(side="left")
            tk.Label(
                head, text=instrument, bg=PANEL_ALT, fg=color, font=("Consolas", 8, "bold")
            ).pack(side="right")

            gain = tk.DoubleVar(value=float(track.get("gain", 0.0) or 0.0))
            pan = tk.DoubleVar(value=float(track.get("pan", 0.0) or 0.0))
            gain_text = tk.StringVar(value=f"{gain.get():.2f}")
            pan_text = tk.StringVar(value=f"{pan.get():+.2f}")
            gain_line = tk.Frame(content, bg=PANEL_ALT)
            gain_line.pack(fill="x", pady=(5, 0))
            tk.Label(gain_line, text="GAIN", bg=PANEL_ALT, fg=MUTED, font=("Consolas", 8)).pack(
                side="left"
            )
            gain_scale = ttk.Scale(
                gain_line,
                from_=0.0,
                to=1.0,
                variable=gain,
                style="Daw.Horizontal.TScale",
            )
            gain_scale.pack(side="left", fill="x", expand=True, padx=6)
            tk.Label(
                gain_line,
                textvariable=gain_text,
                width=5,
                bg=PANEL_ALT,
                fg=TEXT,
                font=("Consolas", 8),
            ).pack(side="right")
            gain_scale.configure(
                command=lambda value, label=gain_text: label.set(f"{float(value):.2f}")
            )
            gain_scale.bind(
                "<ButtonRelease-1>",
                lambda _event, i=index, value=gain: self.commit_gain(i, value.get()),
            )

            pan_line = tk.Frame(content, bg=PANEL_ALT)
            pan_line.pack(fill="x")
            tk.Label(pan_line, text="PAN ", bg=PANEL_ALT, fg=MUTED, font=("Consolas", 8)).pack(
                side="left"
            )
            pan_scale = ttk.Scale(
                pan_line,
                from_=-1.0,
                to=1.0,
                variable=pan,
                style="Daw.Horizontal.TScale",
            )
            pan_scale.pack(side="left", fill="x", expand=True, padx=6)
            tk.Label(
                pan_line,
                textvariable=pan_text,
                width=5,
                bg=PANEL_ALT,
                fg=TEXT,
                font=("Consolas", 8),
            ).pack(side="right")
            pan_scale.configure(
                command=lambda value, label=pan_text: label.set(f"{float(value):+.2f}")
            )
            pan_scale.bind(
                "<ButtonRelease-1>",
                lambda _event, i=index, value=pan: self.commit_pan(i, value.get()),
            )

            mute = ttk.Button(
                head,
                text="M●" if index in self.muted_gains else "M",
                width=3,
                style="Daw.TButton",
                command=lambda i=index: self.toggle_mute(i),
            )
            mute.pack(side="right", padx=(0, 7))
            self.track_controls.append(
                {"gain": gain, "gain_text": gain_text, "mute": mute}
            )

    def commit_gain(self, track: int, gain: float) -> None:
        self.muted_gains.pop(track, None)
        self.send_action("set_track_gain", track=track, gain=gain)
        self.status_var.set(f"Rendering gain revision for track {track + 1}")

    def commit_pan(self, track: int, pan: float) -> None:
        self.send_action("set_track_pan", track=track, pan=pan)
        self.status_var.set(f"Rendering stereo revision for track {track + 1}")

    def toggle_mute(self, track: int) -> None:
        if not 0 <= track < len(self.track_controls):
            return
        control = self.track_controls[track]
        gain: tk.DoubleVar = control["gain"]
        if track in self.muted_gains:
            restored = self.muted_gains.pop(track)
            gain.set(restored)
            control["gain_text"].set(f"{restored:.2f}")
            control["mute"].configure(text="M")
            self.send_action("set_track_gain", track=track, gain=restored)
        else:
            self.muted_gains[track] = max(0.05, gain.get())
            gain.set(0.0)
            control["gain_text"].set("0.00")
            control["mute"].configure(text="M●")
            self.send_action("set_track_gain", track=track, gain=0.0)

    def draw_all(self) -> None:
        self.draw_timeline()
        self.draw_sequencer()

    def close(self) -> None:
        if self.closing:
            return
        self.closing = True
        self.root.destroy()

    def draw_timeline(self) -> None:
        self.timeline.delete("all")
        width = max(300, self.timeline.winfo_width())
        height = max(80, self.timeline.winfo_height())
        if not self.score:
            self.timeline.create_text(width / 2, height / 2, text="MUSICAL FORM", fill=MUTED)
            return
        bars = max(1, int(self.score.get("bars", 1)))
        colors = ("#27364b", "#28505a", "#315f51", "#493958", "#6a5128")
        for index, (name, start, end) in enumerate(self.form_timeline):
            x0, x1 = start / bars * width, end / bars * width
            color = colors[index % len(colors)]
            self.timeline.create_rectangle(x0, 5, x1, 43, fill=color, outline=BACKGROUND)
            if x1 - x0 > 25:
                self.timeline.create_text((x0 + x1) / 2, 24,
                    text=f"{name.upper()}  {start + 1}–{end}" if x1 - x0 > 95 else name.upper(),
                    fill=TEXT, font=("Consolas", 8, "bold"), tags="form_label")
        if not self.form_timeline:
            self.timeline.create_text(width / 2, 24, text="FORM · WAITING FOR SCORE ANALYSIS", fill=MUTED)
        progression = self.score.get("progression", [])
        if isinstance(progression, list) and progression:
            current, bar, chord_bars, cycle_bars = chord_cycle_state(
                self.score, self.position_seconds, self.harmonic_timeline)
            self.timeline.create_text(12, 59, anchor="w", fill=MUTED, font=("Consolas", 8),
                text=f"BAR {bar + 1}/{bars}  ·  {chord_bars} BAR(S) PER CHORD  ·  {cycle_bars}-BAR CYCLE", tags="harmonic_rhythm")
            count = min(len(progression), max(3, int(width / 100)))
            start = min(max(0, current - count // 2), len(progression) - count)
            cell = width / count
            for slot, index in enumerate(range(start, start + count)):
                chord = progression[index]
                if isinstance(chord, dict):
                    selected = index == current
                    self.timeline.create_rectangle(slot * cell + 3, 75, (slot + 1) * cell - 3, 108,
                        fill="#173c43" if selected else PANEL_ALT, outline=CYAN if selected else GRID)
                    self.timeline.create_text(slot * cell + cell / 2, 92,
                        text=f"{'▶ ' if selected else ''}{chord_name(self.score, chord)}",
                        fill=CYAN if selected else TEXT, font=("Consolas", 10, "bold"),
                        tags="current_chord" if selected else "cycle_chord")
        duration = score_duration(self.score)
        x = (self.position_seconds / max(duration, 0.001)) * width
        self.timeline.create_line(x, 2, x, 44, fill=GOLD, width=3)

    def draw_sequencer(self) -> None:
        self.canvas.delete("all")
        width = max(300, self.canvas.winfo_width())
        height = max(240, self.canvas.winfo_height())
        tracks = self.tracks()
        if not tracks:
            self.canvas.create_text(
                width / 2,
                height / 2,
                text="THE ORGANIST IS PREPARING THE SCORE",
                fill=MUTED,
                font=("Consolas", 12),
            )
            return
        label_width = min(210, max(145, width * 0.20))
        top, bottom = 28, 18
        row_height = max(38, (height - top - bottom) / len(tracks))
        longest = max(len(track.get("steps", [])) for track in tracks)
        grid = score_grid(self.score)
        total_columns = max(grid, math.ceil(longest / grid) * grid)
        columns = min(64, total_columns)
        cell_width = max(5, (width - label_width - 12) / columns)
        actual_step = sequencer_column(self.score, self.position_seconds, total_columns, self.form_timeline)
        page_start = (actual_step // columns) * columns
        playhead = actual_step - page_start

        for column in range(columns):
            x = label_width + column * cell_width
            if column % grid == 0:
                self.canvas.create_text(
                    x + 3,
                    13,
                    text=str((page_start + column) // grid + 1),
                    fill=MUTED,
                    anchor="w",
                    font=("Consolas", 7),
                )
            if column == playhead:
                self.canvas.create_rectangle(
                    x, top - 5, x + cell_width, height - bottom, fill="#173547", outline=""
                )

        for row, track in enumerate(tracks):
            color = TRACK_COLORS[row % len(TRACK_COLORS)]
            y = top + row * row_height
            name = str(track.get("name", f"Track {row + 1}"))[:24]
            instrument = str(track.get("instrument") or track.get("waveform", "voice")).replace(
                "_", " "
            )
            gain = max(0.0, min(1.0, float(track.get("gain", 0.0) or 0.0)))
            steps = expressed_steps(track)
            self.canvas.create_text(
                10,
                y + 12,
                text=name,
                fill=TEXT,
                anchor="w",
                font=("Segoe UI", 9, "bold"),
            )
            self.canvas.create_text(
                10,
                y + 29,
                text=instrument.upper(),
                fill=MUTED,
                anchor="w",
                font=("Consolas", 7),
            )
            meter = 46 * gain if self.transport == "playing" and sequencer_note(steps, actual_step, grid) is not None else 0
            self.canvas.create_rectangle(
                label_width - 58,
                y + 10,
                label_width - 8,
                y + 17,
                fill="#182331",
                outline="",
            )
            self.canvas.create_rectangle(
                label_width - 58,
                y + 10,
                label_width - 58 + meter,
                y + 17,
                fill=color,
                outline="",
                tags=f"activity_meter_{row}",
            )
            for column in range(columns):
                x = label_width + column * cell_width
                value = sequencer_note(steps, page_start + column, grid)
                active = value is not None
                inset = 2 if cell_width >= 10 else 1
                self.canvas.create_rectangle(
                    x + inset,
                    y + 5,
                    x + cell_width - inset,
                    y + row_height - 5,
                    fill=color if active else PANEL,
                    outline="#f5fff9" if column == playhead else GRID,
                    width=2 if column == playhead else 1,
                    tags=f"note_cell_{row}_{column}",
                )
                if active and cell_width >= 18:
                    self.canvas.create_text(
                        x + cell_width / 2,
                        y + row_height / 2,
                        text=str(value),
                        fill=BACKGROUND,
                        font=("Consolas", 7, "bold"),
                    )

    def tick(self) -> None:
        try:
            while True:
                self.apply_command(self.commands.get_nowait())
        except queue.Empty:
            pass
        if self.closing:
            return
        self.advance_clock()
        if self.transport in {"playing", "muted"}:
            remaining = max(0, math.ceil(self.next_change_at - time.monotonic()))
            self.countdown_var.set(f"NEXT DEVELOPMENT {remaining:02d}s")
            self.draw_all()
        else:
            self.countdown_var.set("DEVELOPMENT PAUSED")
        self.root.after(80, self.tick)


def claim_single_window(name: str) -> None:
    """Leave an existing studio alone; never terminate a PID from an old file."""
    import tempfile
    from pathlib import Path
    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Sidecars" / "scripts"))
    from studio_lifetime import claim_window_lock
    claim_window_lock(Path(tempfile.gettempdir()) / f".teledra-{name}.lock")


def self_test() -> int:
    assert protocol_action("play") == {"action": "play"}
    for seconds in CADENCES.values():
        assert protocol_action("set_cadence", seconds=seconds)["seconds"] == seconds
    for seconds in (0, 24, 59, 901):
        try:
            protocol_action("set_cadence", seconds=seconds)
        except ValueError:
            pass
        else:
            raise AssertionError("out-of-range cadence accepted")
    assert protocol_action("set_track_gain", track=2, gain=0.42)["gain"] == 0.42
    assert protocol_action("set_track_pan", track=1, pan=-0.25)["pan"] == -0.25
    assert movement_name(6) == "MODULATION"
    assert [sequencer_note([0, 1, 2, 3], i) for i in range(8)] == [0, None, None, None, 1, None, None, None]
    assert sequencer_note(list(range(32)), 16) == 16
    assert sequencer_note([0, 1, 2, 3], 16) == 0
    assert abs(score_duration({"bpm": 120, "bars": 8}) - 16.0) < 0.001
    try:
        protocol_action("shell", command="calc")
    except ValueError:
        pass
    else:
        raise AssertionError("free-form actions must be refused")
    print("organist_daw self-test passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    claim_single_window("organist-daw")
    root = tk.Tk()
    OrganistStudio(root)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

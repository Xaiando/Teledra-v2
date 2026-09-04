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
    "Frequent · 24s": 24,
    "Flowing · 36s": 36,
    "Cinematic · 48s": 48,
}
ALLOWED_ACTIONS = {
    "play",
    "pause",
    "stop",
    "evolve",
    "new_movement",
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
        if not 12 <= seconds <= 120:
            raise ValueError("cadence must be 12..120 seconds")
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
    }
    tonic = int(score.get("tonic", 0) or 0) % 12
    mode = str(score.get("mode", "minor")).lower()
    degree = int(chord.get("root_degree", 0) or 0) % 7
    quality = str(chord.get("quality", "major"))
    suffix = {
        "major": "",
        "minor": "m",
        "diminished": "°",
        "suspended2": "sus2",
        "suspended4": "sus4",
    }.get(quality, "")
    return notes[(tonic + scales.get(mode, scales["minor"])[degree]) % 12] + suffix


class OrganistStudio:
    def __init__(self, root: tk.Tk) -> None:
        self.root = root
        self.commands: queue.Queue[dict[str, object]] = queue.Queue()
        self.score: dict[str, object] = {}
        self.analysis: dict[str, object] = {}
        self.transport = "stopped"
        self.position_seconds = 0.0
        self.last_clock = time.monotonic()
        self.cadence_seconds = 36
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
            text="Verified generative composition · one Rust audio source",
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
            text="MIXER · VERIFIED ON RELEASE",
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
        self.info_var = tk.StringVar(value="Native synth awaiting score")
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
        self.quality_var = tk.StringVar(value="CRAFT --")
        tk.Label(
            top,
            textvariable=self.quality_var,
            bg="#13251f",
            fg=GREEN,
            font=("Consolas", 10, "bold"),
            padx=12,
            pady=9,
        ).pack(side="right", padx=(8, 0))

        self.timeline = tk.Canvas(right, height=92, bg=PANEL, highlightthickness=1, highlightbackground=GRID)
        self.timeline.pack(fill="x", pady=(0, 10))
        self.timeline.bind("<Configure>", lambda _event: self.draw_timeline())
        self.canvas = tk.Canvas(right, bg="#090e17", highlightthickness=1, highlightbackground=GRID)
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<Configure>", lambda _event: self.draw_sequencer())

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
        self.status_var = tk.StringVar(
            value="Mixer changes render off-thread and replace playback only after verification"
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
            lambda _event: self.send_action("pause" if self.transport == "playing" else "play"),
        )
        self.root.bind("<Control-e>", lambda _event: self.send_action("evolve"))
        self.root.bind("<Control-n>", lambda _event: self.send_action("new_movement"))

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
        action = command.get("action")
        if action == "shutdown":
            self.close()
        elif action == "score" and isinstance(command.get("score"), dict):
            self.advance_clock()
            old_names = [str(track.get("name", "")) for track in self.tracks()]
            self.score = command["score"]  # type: ignore[assignment]
            new_names = [str(track.get("name", "")) for track in self.tracks()]
            if old_names and old_names != new_names:
                self.muted_gains.clear()
            analysis = command.get("analysis", {})
            self.analysis = analysis if isinstance(analysis, dict) else {}
            self.transport = str(command.get("transport", "playing"))
            self.position_seconds = 0.0
            self.cadence_seconds = max(
                12, min(120, int(command.get("cadence_seconds", 36) or 36))
            )
            self.last_clock = time.monotonic()
            self.next_change_at = self.last_clock + self.cadence_seconds
            self.sync_cadence_label()
            self.refresh_labels()
            self.rebuild_mixer()
            self.draw_all()
        elif action == "transport":
            self.advance_clock()
            self.transport = str(command.get("state", "stopped"))
            if self.transport == "stopped":
                self.position_seconds = 0.0
            elif self.transport == "playing":
                self.next_change_at = time.monotonic() + self.cadence_seconds
            self.last_clock = time.monotonic()
            self.refresh_labels()
        elif action == "cadence":
            self.cadence_seconds = max(12, min(120, int(command.get("seconds", 36) or 36)))
            self.next_change_at = time.monotonic() + self.cadence_seconds
            self.sync_cadence_label()

    def advance_clock(self) -> None:
        now = time.monotonic()
        if self.transport == "playing" and self.score:
            self.position_seconds = (
                self.position_seconds + now - self.last_clock
            ) % score_duration(self.score)
        self.last_clock = now

    def sync_cadence_label(self) -> None:
        nearest = min(CADENCES, key=lambda label: abs(CADENCES[label] - self.cadence_seconds))
        self.cadence_var.set(nearest)

    def change_cadence(self, _event: object = None) -> None:
        seconds = CADENCES.get(self.cadence_var.get(), 36)
        self.cadence_seconds = seconds
        self.next_change_at = time.monotonic() + seconds
        self.send_action("set_cadence", seconds=seconds)

    def refresh_labels(self) -> None:
        title = str(self.score.get("title", "Court Score"))
        bpm = int(self.score.get("bpm", 0) or 0)
        bars = int(self.score.get("bars", 0) or 0)
        tonic = int(self.score.get("tonic", 0) or 0)
        mode = str(self.score.get("mode", "unknown")).upper()
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
        self.quality_var.set(f"CRAFT {quality:03d}/100 · {grade}")
        checks = (
            ("pulse", "PULSE"),
            ("foundation", "FOUNDATION"),
            ("harmony", "HARMONY"),
            ("melody", "MELODY"),
            ("cadence", "CADENCE"),
            ("breathing_room", "SPACE"),
            ("palette", "PALETTE"),
            ("form", "FORM"),
        )
        summary = "  ".join(
            f"{'✓' if self.analysis.get(key) else '·'} {label}" for key, label in checks
        )
        guidance = self.analysis.get("guidance", [])
        if isinstance(guidance, list) and guidance:
            summary += "   NEXT: " + str(guidance[0]).upper()
        self.craft_var.set(summary)
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
            self.timeline.create_text(width / 2, height / 2, text="ARRANGEMENT ARC", fill=MUTED)
            return
        sections = (
            ("INTRO", 0.00, 0.15, "#27364b"),
            ("RISE", 0.15, 0.35, "#28505a"),
            ("MAIN", 0.35, 0.62, "#315f51"),
            ("BREAK", 0.62, 0.80, "#493958"),
            ("FINALE", 0.80, 1.00, "#6a5128"),
        )
        for name, start, end, color in sections:
            x0, x1 = start * width, end * width
            self.timeline.create_rectangle(x0, 5, x1, 43, fill=color, outline=BACKGROUND)
            self.timeline.create_text(
                (x0 + x1) / 2,
                24,
                text=name,
                fill=TEXT,
                font=("Consolas", 8, "bold"),
            )
        progression = self.score.get("progression", [])
        if isinstance(progression, list) and progression:
            cell = width / len(progression)
            for index, chord in enumerate(progression):
                if isinstance(chord, dict):
                    self.timeline.create_text(
                        index * cell + cell / 2,
                        66,
                        text=chord_name(self.score, chord),
                        fill=CYAN,
                        font=("Consolas", 9, "bold"),
                    )
        duration = score_duration(self.score)
        x = (self.position_seconds / max(duration, 0.001)) * width
        self.timeline.create_line(x, 2, x, height - 2, fill=GOLD, width=3)

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
        columns = min(64, max(len(track.get("steps", [])) for track in tracks))
        columns = max(1, columns)
        cell_width = max(5, (width - label_width - 12) / columns)
        bpm = max(40, int(self.score.get("bpm", 92) or 92))
        seconds_per_bar = 4.0 * 60.0 / bpm
        playhead = int(
            (self.position_seconds % seconds_per_bar) / seconds_per_bar * columns
        ) % columns
        pulse = 0.5 + 0.5 * math.sin(time.monotonic() * 5.5)

        for column in range(columns):
            x = label_width + column * cell_width
            if column % max(1, columns // 4) == 0:
                self.canvas.create_text(
                    x + 3,
                    13,
                    text=str(column + 1),
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
            steps = track.get("steps", [])
            steps = steps if isinstance(steps, list) and steps else [None]
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
            meter = 46 * gain * (
                (0.68 + 0.32 * pulse) if self.transport == "playing" else 0.18
            )
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
            )
            for column in range(columns):
                x = label_width + column * cell_width
                value = steps[column % len(steps)]
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
        if self.transport == "playing":
            remaining = max(0, math.ceil(self.next_change_at - time.monotonic()))
            self.countdown_var.set(f"NEXT DEVELOPMENT {remaining:02d}s")
            self.draw_all()
        else:
            self.countdown_var.set("DEVELOPMENT PAUSED")
        self.root.after(80, self.tick)


def claim_single_window(name: str) -> None:
    """Close an older Organist window before opening a new one."""
    import os
    import signal
    import tempfile
    from pathlib import Path

    record = Path(tempfile.gettempdir()) / f".teledra-{name}.pid"
    try:
        previous = int(record.read_text(encoding="utf-8").strip())
    except (OSError, ValueError):
        previous = None
    if previous and previous != os.getpid():
        try:
            os.kill(previous, signal.SIGTERM)
        except (OSError, PermissionError):
            pass
    try:
        record.write_text(str(os.getpid()), encoding="utf-8")
    except OSError:
        pass


def self_test() -> int:
    assert protocol_action("play") == {"action": "play"}
    assert protocol_action("set_cadence", seconds=24)["seconds"] == 24
    assert protocol_action("set_track_gain", track=2, gain=0.42)["gain"] == 0.42
    assert protocol_action("set_track_pan", track=1, pan=-0.25)["pan"] == -0.25
    assert movement_name(6) == "MODULATION"
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

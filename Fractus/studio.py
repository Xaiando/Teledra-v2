"""Persistent, hot-reloadable presentation surface for verified Fractus art."""

from __future__ import annotations

import argparse
import json
import math
import time
import tkinter as tk
from pathlib import Path
from tkinter import ttk

from PIL import Image, ImageTk


BACKGROUND = "#07110f"
PANEL = "#0d1d19"
EMERALD = "#3ce6ad"
MUTED = "#7ea89a"
MAX_BYTES = 128 * 1024 * 1024
MAX_FRAMES = 120
MAX_DECODED_PIXELS = 40_000_000


class FractusStudio:
    def __init__(self, root: tk.Tk, trusted_root: Path) -> None:
        self.root = root
        self.trusted_root = trusted_root.resolve()
        self.command_path = self.trusted_root / ".fractus-studio-command.json"
        self.status_path = self.trusted_root / ".fractus-studio-status.json"
        self.last_command_id: object = None
        self.source_frames: list[Image.Image] = []
        self.durations: list[int] = []
        self.frame_index = 0
        self.playing = True
        self.live = False
        self.phase = 0.0
        self.next_frame_at = time.monotonic()
        self.photo: ImageTk.PhotoImage | None = None

        root.title("Fractus // Living Gallery")
        root.geometry("800x980+1100+40")
        root.minsize(500, 620)
        root.configure(bg=BACKGROUND)
        root.protocol("WM_DELETE_WINDOW", root.destroy)

        style = ttk.Style(root)
        style.theme_use("clam")
        style.configure("Studio.TButton", background=PANEL, foreground=EMERALD)

        header = tk.Frame(root, bg=BACKGROUND, padx=18, pady=14)
        header.pack(fill="x")
        tk.Label(
            header,
            text="FRACTUS",
            bg=BACKGROUND,
            fg=EMERALD,
            font=("Consolas", 20, "bold"),
        ).pack(side="left")
        self.title_var = tk.StringVar(value="Waiting for the Artist")
        tk.Label(
            header,
            textvariable=self.title_var,
            bg=BACKGROUND,
            fg="#d8fff0",
            font=("Segoe UI", 11),
        ).pack(side="left", padx=(18, 0))

        controls = tk.Frame(header, bg=BACKGROUND)
        controls.pack(side="right")
        self.play_button = ttk.Button(
            controls, text="Pause", style="Studio.TButton", command=self.toggle_play
        )
        self.play_button.pack(side="left", padx=4)
        ttk.Button(
            controls, text="Fit", style="Studio.TButton", command=self.draw_current
        ).pack(side="left", padx=4)

        self.canvas = tk.Canvas(root, bg="#020806", highlightthickness=1, highlightbackground="#173b31")
        self.canvas.pack(fill="both", expand=True, padx=18, pady=(0, 12))
        self.canvas.bind("<Configure>", lambda _event: self.draw_current())

        footer = tk.Frame(root, bg=PANEL, padx=18, pady=10)
        footer.pack(fill="x")
        self.status_var = tk.StringVar(value="Studio ready")
        tk.Label(
            footer,
            textvariable=self.status_var,
            anchor="w",
            bg=PANEL,
            fg=MUTED,
            font=("Consolas", 10),
        ).pack(fill="x")

        print(json.dumps({"event": "ready"}), flush=True)
        root.after(20, self.tick)

    def read_command_file(self) -> dict[str, object] | None:
        """Returns one new command from the runtime's durable mailbox.

        Windows virtual-environment launchers are redirector processes. Their
        stdin/stdout pipes do not reliably reach the real Python GUI child, so
        a tiny on-disk mailbox is used as the authoritative IPC channel.
        """
        try:
            value = json.loads(self.command_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return None
        if not isinstance(value, dict):
            return None
        command_id = value.get("command_id")
        if command_id is None or command_id == self.last_command_id:
            return None
        self.last_command_id = command_id
        return value

    def write_status(self, event: str, command: dict[str, object], **details: object) -> None:
        status = {
            "event": event,
            "command_id": command.get("command_id"),
            **details,
        }
        temporary = self.status_path.with_suffix(".tmp")
        try:
            temporary.write_text(json.dumps(status), encoding="utf-8")
            temporary.replace(self.status_path)
        except OSError:
            pass

    def validate_path(self, raw_path: object) -> Path:
        path = Path(str(raw_path)).resolve(strict=True)
        if path.suffix.lower() not in {".png", ".gif"}:
            raise ValueError("Fractus Studio accepts only PNG and GIF artifacts")
        if self.trusted_root != path and self.trusted_root not in path.parents:
            raise ValueError("artifact is outside the trusted Fractus directory")
        if path.stat().st_size > MAX_BYTES:
            raise ValueError("artifact exceeds the Studio size limit")
        return path

    def load(self, command: dict[str, object]) -> None:
        path = self.validate_path(command.get("path", ""))
        frames: list[Image.Image] = []
        durations: list[int] = []
        with Image.open(path) as image:
            count = min(getattr(image, "n_frames", 1), MAX_FRAMES)
            if image.width < 1 or image.height < 1 or image.width * image.height * count > MAX_DECODED_PIXELS:
                raise ValueError("artifact exceeds the Studio decoded-frame limit")
            for index in range(count):
                image.seek(index)
                frames.append(image.convert("RGBA").copy())
                durations.append(max(30, min(500, int(image.info.get("duration", 70)))))
        if not frames:
            raise ValueError("artifact contains no visible frames")
        self.source_frames = frames
        self.durations = durations
        self.frame_index = 0
        self.phase = 0.0
        self.next_frame_at = time.monotonic()
        self.live = bool(command.get("live", len(frames) > 1))
        self.playing = True
        self.play_button.configure(text="Pause")
        self.title_var.set(path.stem.replace("fractus-", "Scene "))
        kind = "animated render" if len(frames) > 1 else "living still"
        self.status_var.set(f"{kind}  |  {len(frames)} frame(s)  |  {frames[0].width} x {frames[0].height}")
        self.draw_current()
        self.reveal()
        self.write_status("loaded", command, path=str(path), frames=len(frames))
        print(json.dumps({"event": "loaded", "path": str(path), "frames": len(frames)}), flush=True)

    def reveal(self) -> None:
        """Places new art above the fullscreen court long enough to be noticed."""
        self.root.deiconify()
        self.root.lift()
        self.root.attributes("-topmost", True)
        self.root.after(1500, lambda: self.root.attributes("-topmost", False))

    def apply_command(self, command: dict[str, object]) -> bool:
        if command.get("action") == "shutdown":
            self.root.destroy()
            return False
        if command.get("action") == "show":
            self.load(command)
        return True

    def toggle_play(self) -> None:
        self.playing = not self.playing
        self.play_button.configure(text="Pause" if self.playing else "Play")
        self.next_frame_at = time.monotonic()

    def draw_current(self) -> None:
        if not self.source_frames:
            self.canvas.delete("all")
            self.canvas.create_text(
                max(1, self.canvas.winfo_width()) // 2,
                max(1, self.canvas.winfo_height()) // 2,
                text="The gallery is waiting for Fractus art",
                fill=MUTED,
                font=("Consolas", 13),
            )
            return
        width = max(64, self.canvas.winfo_width())
        height = max(64, self.canvas.winfo_height())
        source = self.source_frames[self.frame_index]
        base_scale = min(width / source.width, height / source.height)
        # A static render becomes a subtle living canvas; GIFs preserve the renderer's
        # phase animation and receive only a much smaller cinematic drift.
        amplitude = 0.025 if len(self.source_frames) == 1 else 0.006
        scale = max(0.01, base_scale * (1.0 + amplitude * (1.0 + math.sin(self.phase))))
        target = (max(1, round(source.width * scale)), max(1, round(source.height * scale)))
        rendered = source.resize(target, Image.Resampling.LANCZOS)
        self.photo = ImageTk.PhotoImage(rendered)
        self.canvas.delete("all")
        drift = 5.0 * math.sin(self.phase * 0.57)
        self.canvas.create_image(width / 2 + drift, height / 2 - drift * 0.45, image=self.photo)

    def tick(self) -> None:
        command: dict[str, object] | None = None
        try:
            command = self.read_command_file()
            if command is not None and not self.apply_command(command):
                return
        except Exception as error:  # keep the persistent window alive after a bad command
            self.status_var.set(f"Could not load art: {error}")
            if command is not None:
                self.write_status("error", command, message=str(error))
            print(json.dumps({"event": "error", "message": str(error)}), flush=True)

        now = time.monotonic()
        if self.playing and self.source_frames:
            self.phase += 0.055
            if len(self.source_frames) > 1 and now >= self.next_frame_at:
                self.frame_index = (self.frame_index + 1) % len(self.source_frames)
                self.next_frame_at = now + self.durations[self.frame_index] / 1000.0
            self.draw_current()
        self.root.after(50, self.tick)


def claim_single_window(root: Path, name: str) -> None:
    """Closes an older window of this studio before opening a new one.

    The court launches these through the virtual environment's ``python.exe``,
    which on Windows re-executes the real interpreter as a child. The process
    the runtime holds is therefore the stub, and stopping it can leave the
    window it spawned running with no parent. Windows then accumulate one per
    render. Recording the live window here, at the layer that owns it, closes
    the previous one no matter which process launched it.
    """
    import os
    import signal

    record = root / f".{name}.pid"
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
        record.parent.mkdir(parents=True, exist_ok=True)
        record.write_text(str(os.getpid()), encoding="utf-8")
    except OSError:
        pass


def main() -> int:
    parser = argparse.ArgumentParser(description="Teledra's persistent Fractus gallery")
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=True)
    claim_single_window(args.root, "fractus-studio")
    root = tk.Tk()
    FractusStudio(root, args.root)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

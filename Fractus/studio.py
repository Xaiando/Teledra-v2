"""Persistent gallery for Fractus stills and true-colour, looping animation."""

from __future__ import annotations

import argparse
from bisect import bisect_right
from collections import OrderedDict
from collections.abc import Callable
import json
import math
import re
import time
import tkinter as tk
from pathlib import Path
from tkinter import ttk

from PIL import Image, ImageTk


BACKGROUND = "#090e14"
PANEL = "#101921"
EMERALD = "#9ce5ce"
MUTED = "#7c949e"
MAX_BYTES = 128 * 1024 * 1024
MAX_FRAMES = 240
MAX_DECODED_PIXELS = 96_000_000
MAX_CACHE_PIXELS = 12_000_000
MAX_RENDER_PIXELS = 8_000_000
MAX_MANIFEST_BYTES = 256 * 1024
SPEEDS = (0.5, 1.0, 1.5)


def display_text(value: object) -> str:
    """Metadata is only bounded, printable label text, never executable markup."""
    if not isinstance(value, str):
        return ""
    printable = "".join(character if character.isprintable() else " " for character in value[:500])
    return " ".join(printable.split())[:100].rstrip()


def artwork_title(path: Path, title: object = None) -> str:
    """Prefer an artwork name or scene metadata over an internal artifact ID."""
    supplied = display_text(title)
    if supplied:
        return supplied
    try:
        manifest_path = path.with_suffix(".manifest.json").resolve(strict=True)
        # Read only the companion file; a redirected metadata symlink must not
        # turn unrelated files into gallery labels.
        if manifest_path.parent == path.resolve().parent:
            with manifest_path.open("rb") as handle:
                encoded = handle.read(MAX_MANIFEST_BYTES + 1)
            if len(encoded) <= MAX_MANIFEST_BYTES:
                manifest = json.loads(encoded)
                scene = manifest.get("scene") if isinstance(manifest, dict) else None
                if isinstance(scene, dict):
                    kind = display_text(scene.get("kind")).replace("_", " ").title()
                    palette = display_text(scene.get("palette")).replace("_", " ").title()
                    if kind:
                        return display_text(f"{kind} · {palette}" if palette else kind)
    except (OSError, ValueError, UnicodeError, RecursionError):
        pass
    stem = re.sub(r"^fractus(?:[-_ ]|$)", "", path.stem, flags=re.IGNORECASE)
    if not stem or not any(character.isalpha() for character in stem) or re.fullmatch(r"\d{4}[-_]?\d{2}[-_]?\d{2}[Tt].+", stem):
        return "Geometric study"
    stem = re.sub(r"^\d{1,3}[-_ ]+(?=[A-Za-z])", "", stem)
    return display_text(stem.replace("_", " ").replace("-", " ").title()) or "Geometric study"


def fit_size(source: tuple[int, int], viewport: tuple[int, int]) -> tuple[int, int]:
    """Fit the whole artwork without cropping, distortion or artificial motion."""
    width, height = source
    available_width, available_height = (max(1, value) for value in viewport)
    scale = min(available_width / width, available_height / height,
                math.sqrt(MAX_RENDER_PIXELS / (width * height)))
    return max(1, int(width * scale)), max(1, int(height * scale))


def decode_artifact(path: Path) -> tuple[list[Image.Image], list[float]]:
    """Decode composited APNG/GIF frames while bounding resident source memory.

    An APNG may have a poster image outside its animation. Pillow exposes that
    as frame zero, so it must not be included in the playback timeline.
    """
    frames: list[Image.Image] = []
    durations: list[float] = []
    with Image.open(path) as source:
        start = 1 if source.info.get("default_image", False) else 0
        count = getattr(source, "n_frames", 1) - start
        if count < 1:
            raise ValueError("artifact contains no visible frames")
        if count > MAX_FRAMES:
            raise ValueError(f"artifact exceeds the Studio limit of {MAX_FRAMES} frames")
        if source.width < 1 or source.height < 1 or source.width * source.height * count > MAX_DECODED_PIXELS:
            raise ValueError("artifact exceeds the Studio decoded-frame limit")
        for index in range(start, count + start):
            source.seek(index)
            frames.append(source.convert("RGBA"))
            try:
                duration = float(source.info.get("duration", 100.0))
            except (TypeError, ValueError):
                duration = 100.0
            durations.append(duration if math.isfinite(duration) and duration > 0 else 100.0)
    return frames, durations


class PlaybackClock:
    """Media time advances from elapsed wall time, including missed UI ticks."""

    def __init__(self, durations: list[float], now: float, *, playing: bool = True, speed: float = 1.0) -> None:
        if not durations or any(not math.isfinite(value) or value <= 0 for value in durations):
            raise ValueError("frame durations must be positive and finite")
        if not math.isfinite(speed) or speed <= 0:
            raise ValueError("playback speed must be positive and finite")
        self.ends: list[float] = []
        total = 0.0
        for duration in durations:
            total += duration / 1000.0
            self.ends.append(total)
        self.duration = total
        self.anchor = now
        self.offset = 0.0
        self.playing = playing
        self.speed = speed

    def position(self, now: float) -> float:
        elapsed = max(0.0, now - self.anchor) * self.speed if self.playing else 0.0
        return (self.offset + elapsed) % self.duration

    def frame_at(self, now: float) -> int:
        return min(bisect_right(self.ends, self.position(now)), len(self.ends) - 1)

    def set_playing(self, playing: bool, now: float) -> None:
        self.offset = self.position(now)
        self.anchor = now
        self.playing = playing

    def set_speed(self, speed: float, now: float) -> None:
        if not math.isfinite(speed) or speed <= 0:
            raise ValueError("playback speed must be positive and finite")
        self.offset = self.position(now)
        self.anchor = now
        self.speed = speed

    def seek_frame(self, index: int, now: float) -> None:
        index %= len(self.ends)
        self.offset = self.ends[index - 1] if index else 0.0
        self.anchor = now

    def next_delay_ms(self, now: float) -> int:
        if not self.playing or len(self.ends) == 1:
            return 100
        remaining = self.ends[self.frame_at(now)] - self.position(now)
        return max(1, min(100, math.ceil(remaining * 1000.0 / self.speed)))


class FractusStudio:
    def __init__(self, root: tk.Tk, trusted_root: Path) -> None:
        self.root = root
        self.trusted_root = trusted_root.resolve()
        self.command_path = self.trusted_root / ".fractus-studio-command.json"
        self.status_path = self.trusted_root / ".fractus-studio-status.json"
        self.last_command_id: object = None
        self.source_frames: list[Image.Image] = []
        self.durations: list[float] = []
        self.frame_index = 0
        self.playing = True
        self.speed = 1.0
        self.clock: PlaybackClock | None = None
        self.photo: ImageTk.PhotoImage | None = None
        self.image_item: int | None = None
        self.render_cache: OrderedDict[tuple[object, ...], Image.Image] = OrderedDict()
        self.cache_pixels = 0
        self.last_draw_key: tuple[object, ...] | None = None
        self.revealed = False
        self.fullscreen = False
        self.resize_job: str | None = None
        self.next_mailbox_at = 0.0

        root.title("Fractus / Motion Gallery")
        root.geometry("900x980+60+40")
        root.minsize(500, 520)
        root.configure(bg=BACKGROUND)
        root.protocol("WM_DELETE_WINDOW", root.destroy)

        style = ttk.Style(root)
        style.theme_use("clam")
        style.configure("Studio.TButton", background=PANEL, foreground=EMERALD,
                        borderwidth=0, padding=(12, 8), font=("Segoe UI", 10))
        style.map("Studio.TButton", background=[("active", "#213c41")],
                  foreground=[("disabled", "#43565d")])

        self.header = tk.Frame(root, bg=BACKGROUND, padx=22, pady=16)
        self.header.pack(fill="x")
        brand = tk.Frame(self.header, bg=BACKGROUND)
        brand.pack(side="left")
        tk.Label(brand, text="F R A C T U S", bg=BACKGROUND, fg=EMERALD,
                 font=("Segoe UI", 17, "bold"), anchor="w").pack(anchor="w")
        tk.Label(brand, text="M O T I O N   G A L L E R Y", bg=BACKGROUND, fg=MUTED,
                 font=("Segoe UI", 8), anchor="w").pack(anchor="w", pady=(3, 0))
        self.title_var = tk.StringVar(value="Waiting for the Artist")
        tk.Label(self.header, textvariable=self.title_var, bg=BACKGROUND,
                 fg="#d7e7e5", font=("Segoe UI", 11), anchor="e").pack(side="right")

        self.canvas = tk.Canvas(root, bg="#04070c", highlightthickness=1,
                                highlightbackground="#233037")
        self.canvas.pack(fill="both", expand=True, padx=20, pady=(0, 12))
        self.canvas.bind("<Configure>", self.on_resize)
        self.canvas.bind("<Double-Button-1>", lambda _event: self.toggle_fullscreen())
        self.empty_item = self.canvas.create_text(0, 0,
            text="Geometry in motion\n\nWaiting for a Fractus composition",
            justify="center", fill=MUTED, font=("Segoe UI", 13))

        self.footer = tk.Frame(root, bg=BACKGROUND, padx=20, pady=12)
        self.footer.pack(fill="x")
        controls = tk.Frame(self.footer, bg=BACKGROUND)
        controls.pack(fill="x")
        self.play_button = ttk.Button(controls, text="Pause", style="Studio.TButton",
                                      command=self.toggle_play, state="disabled")
        self.play_button.pack(side="left", padx=(0, 6))
        self.restart_button = ttk.Button(controls, text="Restart", style="Studio.TButton",
                                         command=self.restart, state="disabled")
        self.restart_button.pack(side="left", padx=6)
        self.speed_button = ttk.Button(controls, text="1× speed", style="Studio.TButton",
                                       command=self.cycle_speed)
        self.speed_button.pack(side="left", padx=6)
        self.fullscreen_button = ttk.Button(controls, text="Present ↗", style="Studio.TButton",
                                            command=self.toggle_fullscreen)
        self.fullscreen_button.pack(side="right")
        self.status_var = tk.StringVar(value="Space  play / pause     ← →  inspect frames     F11  present")
        tk.Label(self.footer, textvariable=self.status_var, anchor="w", bg=BACKGROUND,
                 fg=MUTED, font=("Segoe UI", 9)).pack(fill="x", pady=(12, 0))
        for key, callback in (("<space>", self.toggle_play), ("<Left>", lambda: self.step(-1)),
                              ("<Right>", lambda: self.step(1)), ("<Home>", self.restart),
                              ("<F11>", self.toggle_fullscreen), ("<Escape>", self.exit_fullscreen)):
            root.bind_class("FractusShortcuts", key,
                            lambda _event, action=callback: self.on_key(action))
        self.bind_shortcuts(root)

        print(json.dumps({"event": "ready"}), flush=True)
        root.after(20, self.tick)

    @staticmethod
    def on_key(action: Callable[[], None]) -> str:
        action()
        return "break"

    def bind_shortcuts(self, widget: tk.Misc) -> None:
        # Consume gallery shortcuts before a focused ttk button can interpret
        # Space itself and trigger a second action or change the speed.
        widget.bindtags(("FractusShortcuts", *widget.bindtags()))
        for child in widget.winfo_children():
            self.bind_shortcuts(child)

    def read_command_file(self) -> dict[str, object] | None:
        """Read the runtime's durable mailbox, avoiding Windows launcher pipes."""
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
        status = {"event": event, "command_id": command.get("command_id"), **details}
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
        frames, durations = decode_artifact(path)
        self.source_frames = frames
        self.durations = durations
        self.frame_index = 0
        self.clock = PlaybackClock(durations, time.monotonic(), playing=self.playing, speed=self.speed)
        self.clear_cache()
        self.last_draw_key = None
        self.title_var.set(artwork_title(path, command.get("title")))
        animated = len(frames) > 1
        for button in (self.play_button, self.restart_button):
            button.configure(state="normal" if animated else "disabled")
        self.update_play_label()
        kind = "Loop" if animated else "Still composition"
        timing = f"  ·  {self.clock.duration:.1f}s  ·  {len(frames) / self.clock.duration:.1f} fps" if animated else ""
        self.status_var.set(f"{kind}{timing}  ·  {frames[0].width} × {frames[0].height}  ·  F11 to present")
        self.draw_current()
        if not self.revealed:
            self.revealed = True
            self.reveal()
        self.write_status("loaded", command, path=str(path), frames=len(frames))
        print(json.dumps({"event": "loaded", "path": str(path), "frames": len(frames)}), flush=True)

    def reveal(self) -> None:
        """Reveal the first composition; subsequent evolution reloads stay quiet."""
        self.root.deiconify()
        self.root.lift()
        self.root.attributes("-topmost", True)
        self.root.after(1000, lambda: self.root.attributes("-topmost", False))

    def apply_command(self, command: dict[str, object]) -> bool:
        if command.get("action") == "shutdown":
            self.root.destroy()
            return False
        if command.get("action") == "show":
            self.load(command)
        return True

    def update_play_label(self) -> None:
        self.play_button.configure(text="Pause" if self.playing else "Play")

    def toggle_play(self) -> None:
        if self.clock is None or len(self.source_frames) < 2:
            return
        now = time.monotonic()
        self.frame_index = self.clock.frame_at(now)
        self.playing = not self.playing
        self.clock.set_playing(self.playing, now)
        self.update_play_label()
        self.draw_current()

    def cycle_speed(self) -> None:
        self.speed = SPEEDS[(SPEEDS.index(self.speed) + 1) % len(SPEEDS)]
        if self.clock is not None:
            self.clock.set_speed(self.speed, time.monotonic())
        self.speed_button.configure(text=f"{self.speed:g}× speed")

    def restart(self) -> None:
        if self.clock is not None:
            self.clock.seek_frame(0, time.monotonic())
            self.frame_index = 0
            self.draw_current()

    def step(self, amount: int) -> None:
        if self.clock is None or len(self.source_frames) < 2:
            return
        now = time.monotonic()
        self.frame_index = (self.clock.frame_at(now) + amount) % len(self.source_frames)
        self.playing = False
        self.clock.set_playing(False, now)
        self.clock.seek_frame(self.frame_index, now)
        self.update_play_label()
        self.draw_current()

    def toggle_fullscreen(self) -> None:
        self.fullscreen = not self.fullscreen
        self.root.attributes("-fullscreen", self.fullscreen)
        if self.fullscreen:
            self.header.pack_forget()
            self.footer.pack_forget()
            self.canvas.pack_configure(padx=0, pady=0)
            self.canvas.configure(highlightthickness=0, cursor="none")
        else:
            self.header.pack(before=self.canvas, fill="x")
            self.footer.pack(after=self.canvas, fill="x")
            self.canvas.pack_configure(padx=20, pady=(0, 12))
            self.canvas.configure(highlightthickness=1, cursor="")

    def exit_fullscreen(self) -> None:
        if self.fullscreen:
            self.toggle_fullscreen()

    def on_resize(self, _event: object = None) -> None:
        if self.resize_job is not None:
            self.root.after_cancel(self.resize_job)
        self.resize_job = self.root.after(80, self.finish_resize)

    def finish_resize(self) -> None:
        self.resize_job = None
        self.clear_cache()
        self.last_draw_key = None
        self.draw_current()

    def clear_cache(self) -> None:
        self.render_cache.clear()
        self.cache_pixels = 0

    def render_frame(self, target: tuple[int, int]) -> Image.Image:
        # Animation uses the fast resampler; paused inspection uses Lanczos.
        # The bounded LRU helps frame stepping without retaining another full
        # decoded animation or allowing large window sizes to exhaust memory.
        resampling = Image.Resampling.BILINEAR if self.playing and len(self.source_frames) > 1 else Image.Resampling.LANCZOS
        key = (self.frame_index, target, resampling)
        if key in self.render_cache:
            self.render_cache.move_to_end(key)
            return self.render_cache[key]
        source = self.source_frames[self.frame_index]
        rendered = source if source.size == target else source.resize(target, resampling)
        if rendered is not source:
            pixels = target[0] * target[1]
            while self.render_cache and self.cache_pixels + pixels > MAX_CACHE_PIXELS:
                _, old = self.render_cache.popitem(last=False)
                self.cache_pixels -= old.width * old.height
            if pixels <= MAX_CACHE_PIXELS:
                self.render_cache[key] = rendered
                self.cache_pixels += pixels
        return rendered

    def draw_current(self) -> None:
        width, height = max(1, self.canvas.winfo_width()), max(1, self.canvas.winfo_height())
        if not self.source_frames:
            self.canvas.coords(self.empty_item, width / 2, height / 2)
            return
        # A frame remains still until its source animation advances. Canvas
        # items and the Tk image are reused instead of deleted every timer tick.
        target = fit_size(self.source_frames[self.frame_index].size, (width, height))
        key = (self.frame_index, target, self.playing, width, height)
        if key == self.last_draw_key:
            return
        rendered = self.render_frame(target)
        if self.photo is None or (self.photo.width(), self.photo.height()) != target:
            self.photo = ImageTk.PhotoImage(rendered)
        else:
            self.photo.paste(rendered)
        self.canvas.itemconfigure(self.empty_item, state="hidden")
        if self.image_item is None:
            self.image_item = self.canvas.create_image(width / 2, height / 2, image=self.photo)
        else:
            self.canvas.itemconfigure(self.image_item, image=self.photo)
            self.canvas.coords(self.image_item, width / 2, height / 2)
        self.last_draw_key = key

    def tick(self) -> None:
        command: dict[str, object] | None = None
        now = time.monotonic()
        if now >= self.next_mailbox_at:
            self.next_mailbox_at = now + 0.1
            try:
                command = self.read_command_file()
                if command is not None and not self.apply_command(command):
                    return
            except Exception as error:  # a malformed render must not close the gallery
                self.status_var.set(f"Could not load art: {error}")
                if command is not None:
                    self.write_status("error", command, message=str(error))
                print(json.dumps({"event": "error", "message": str(error)}), flush=True)

        now = time.monotonic()
        if self.clock is not None and self.playing and len(self.source_frames) > 1:
            self.frame_index = self.clock.frame_at(now)
            self.draw_current()
        # Re-read the clock after rendering: render time belongs to the frame
        # budget and must never accumulate into a slower animation.
        delay = self.clock.next_delay_ms(time.monotonic()) if self.clock else 100
        self.root.after(delay, self.tick)


def claim_single_window(root: Path, name: str) -> None:
    """One studio per data root, with ownership released on any process exit."""
    import sys
    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Sidecars" / "scripts"))
    from studio_lifetime import claim_window_lock
    claim_window_lock(root / f".{name}.lock")


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

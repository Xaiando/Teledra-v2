"""Exercise real APNG playback in a withdrawn Tk window without live IPC.

Usage: python Fractus/probe_studio.py path/to/animation.png
"""

import argparse
import json
from pathlib import Path
import tempfile
import time
import tkinter as tk

from studio import FractusStudio


def pump(root: tk.Tk, seconds: float) -> float:
    start = time.monotonic()
    while time.monotonic() - start < seconds:
        root.update()
        time.sleep(0.002)
    return time.monotonic() - start


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    args = parser.parse_args()
    path = args.path.resolve(strict=True)
    with tempfile.TemporaryDirectory() as directory:
        root = tk.Tk()
        root.withdraw()
        try:
            app = FractusStudio(root, path.parent)
            # Keep the probe's command/status channel away from live files.
            app.command_path = Path(directory) / "command.json"
            app.status_path = Path(directory) / "status.json"
            app.reveal = lambda: None
            # A withdrawn window has no useful measured geometry. Render the
            # actual Tk image at the normal 900-pixel gallery viewport size.
            app.canvas.winfo_width = lambda: 900
            app.canvas.winfo_height = lambda: 900
            started = time.monotonic()
            app.load({"path": str(path), "command_id": "probe", "action": "show"})
            load_seconds = time.monotonic() - started
            assert len(app.source_frames) > 1, "Probe requires an actual animated artifact"
            assert app.clock is not None
            app.restart()
            first_index = app.frame_index
            photo_name = str(app.photo)
            image_item = app.image_item
            elapsed = pump(root, 0.55)
            advanced = (app.frame_index - first_index) % len(app.source_frames)
            expected = elapsed * len(app.source_frames) / app.clock.duration
            assert abs(advanced - expected) <= 2, (advanced, expected)
            app.toggle_play()
            paused_index = app.frame_index
            paused_position = app.clock.position(time.monotonic())
            pump(root, 0.25)
            assert app.frame_index == paused_index
            assert app.clock.position(time.monotonic()) == paused_position
            assert str(app.photo) == photo_name and app.image_item == image_item
            app.step(1)
            assert app.frame_index == (paused_index + 1) % len(app.source_frames)
            app.cycle_speed()
            assert app.clock.speed == 1.5 and not app.clock.playing
            assert root.state() == "withdrawn", "Probe must never take foreground"
            print(json.dumps({
                "result": "passed",
                "artifact": str(path),
                "title": app.title_var.get(),
                "frames": len(app.source_frames),
                "loop_seconds": round(app.clock.duration, 4),
                "load_seconds": round(load_seconds, 3),
                "elapsed_seconds": round(elapsed, 3),
                "frames_advanced": advanced,
                "expected_frames": round(expected, 2),
                "pause_stable": True,
                "photo_reused": True,
                "canvas_items": len(app.canvas.find_all()),
                "cache_pixels": app.cache_pixels,
                "window_visible": False,
            }), flush=True)
        finally:
            root.destroy()


if __name__ == "__main__":
    main()

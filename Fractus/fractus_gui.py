"""Fractus v2 native desktop geometry studio and headless renderer.

Legacy examples remain valid::

    python fractus_gui.py --type julia --iterations 180 --palette purple_haze \
        --c-real -0.78 --c-imag 0.16 --save output/julia.png

The v2 live-code editor compiles with Ctrl+Enter.  Rendering happens on a worker
thread, while every tkinter/ImageTk operation remains on the main thread.
"""

from __future__ import annotations

import argparse
from dataclasses import replace
import json
from pathlib import Path
import queue
import sys
import threading
import time
import tkinter as tk
from tkinter import filedialog, messagebox, ttk
from typing import Any

from PIL import Image, ImageTk

from fractus_dsl import format_live_code, mutate_scene, parse_live_code
from fractus_model import Scene, SceneError, scene_for_frame
from fractus_protocol import (
    COMMAND_PATH,
    OUTPUT_DIR,
    STATUS_PATH,
    CommandEnvelope,
    atomic_write_json,
    command_file_stem,
    command_has_terminal_status,
    command_token,
    make_status,
    read_changed_command,
    write_status,
)
from fractus_registry import (
    FAMILY_SPECS,
    PALETTES,
    capability_manifest,
    default_layer,
    families_by_category,
    legacy_scene,
    validate_scene,
)
from fractus_render import (
    RenderCancelled,
    RenderResult,
    render_scene,
    save_render_result,
    render_animated_gif,
)


BG = "#0c0418"
PANEL = "#16082c"
FIELD = "#231242"
CYAN = "#00e5ff"
MAGENTA = "#ff007f"
VIOLET = "#b500ff"
TEXT = "#e8ddff"
MUTED = "#977db8"


def _scene_from_args(args: argparse.Namespace) -> Scene:
    if args.script and args.code:
        raise SceneError("use either --script or --code, not both")
    if args.script:
        source = Path(args.script).read_text(encoding="utf-8")
        scene = parse_live_code(source)
    elif args.code:
        scene = parse_live_code(args.code)
    else:
        scene = legacy_scene(
            family=args.type or "mandelbrot",
            iterations=args.iterations if args.iterations is not None else 150,
            palette=args.palette or "purple_haze",
            c_real=args.c_real if args.c_real is not None else -0.7,
            c_imag=args.c_imag if args.c_imag is not None else 0.27015,
            seed=args.seed if args.seed is not None else 1,
            width=args.width,
            height=args.height,
        )
    if args.mutate is not None:
        scene = mutate_scene(scene, args.mutate)
    return validate_scene(scene)


def merge_legacy_first_layer(scene: Scene, migrated: Scene) -> tuple[Scene, int]:
    """Apply Quick-tab values to layer 0 without destroying the composition."""

    old_first = scene.layers[0]
    migrated_first = replace(
        migrated.layers[0],
        alpha=old_first.alpha,
        blend=old_first.blend,
        visible=old_first.visible,
    )
    retained_animations = []
    removed_animations = 0
    new_spec = FAMILY_SPECS[migrated_first.family]
    for animation in scene.animations:
        target = animation.target.split(".", 1)
        if len(target) == 2 and target[0] == "0":
            parameter = new_spec.params.get(target[1])
            try:
                compatible = parameter is not None and parameter.kind == "float"
                if compatible:
                    parameter.coerce(animation.start, "animation start")
                    parameter.coerce(animation.end, "animation end")
            except SceneError:
                compatible = False
            if not compatible:
                removed_animations += 1
                continue
        retained_animations.append(animation)
    merged = validate_scene(
        replace(
            scene,
            seed=migrated.seed,
            palette=migrated.palette,
            layers=(migrated_first, *scene.layers[1:]),
            animations=tuple(retained_animations),
        )
    )
    return merged, removed_animations


def _headless_status_path(args: argparse.Namespace) -> Path | None:
    return Path(args.status).expanduser() if args.status else None


def run_headless(args: argparse.Namespace) -> int:
    status_path = _headless_status_path(args)
    command_id = "headless"
    try:
        if args.capabilities:
            print(json.dumps(capability_manifest(), indent=2, sort_keys=True))
            return 0
        if args.self_test:
            summaries = []
            for family in sorted(FAMILY_SPECS):
                scene = validate_scene(
                    Scene(
                        width=96,
                        height=96,
                        seed=20260710,
                        palette="twilight",
                        layers=(default_layer(family),),
                        name=f"Self-test {family}",
                    )
                )
                result = render_scene(scene)
                summaries.append(
                    {
                        "family": family,
                        "hash": result.render_hash[:16],
                        "duration_ms": result.duration_ms,
                        "non_background_fraction": result.metrics[
                            "non_background_fraction"
                        ],
                    }
                )
            print(json.dumps({"state": "passed", "families": summaries}, indent=2))
            return 0

        scene = _scene_from_args(args)
        if args.validate:
            print(scene.canonical_json())
            return 0
        result = render_scene(scene, frame_index=args.frame, fps=args.fps)
        requested = args.output or args.save
        target = (
            Path(requested)
            if requested
            else OUTPUT_DIR / f"fractus_{scene.recipe_hash()[:12]}.png"
        )
        saved = save_render_result(result, scene, target)
        payload = make_status(
            command_id,
            "completed",
            source="headless",
            result=result,
            output_path=saved,
        )
        if status_path:
            atomic_write_json(status_path, payload)
        print(json.dumps(payload, indent=2, sort_keys=True))

        # Autonomous: if animations present (court-controlled creative output),
        # also produce GIF without user intervention.
        if scene.animations:
            try:
                gif = render_animated_gif(scene, num_frames=24, fps=12)
                gif_payload = make_status(
                    command_id,
                    "completed",
                    source="headless-animation",
                    output_path=gif,
                    detail="auto animated GIF for court",
                )
                if status_path:
                    atomic_write_json(status_path.with_suffix('.anim.json'), gif_payload)
                print(json.dumps(gif_payload, indent=2, sort_keys=True))
            except Exception as gexc:
                print(f"Auto GIF failed: {gexc}", file=sys.stderr)
        return 0
    except Exception as exc:
        payload = make_status(
            command_id,
            "rejected",
            source="headless",
            detail=str(exc),
        )
        if status_path:
            try:
                atomic_write_json(status_path, payload)
            except OSError:
                pass
        print(f"Fractus headless error: {exc}", file=sys.stderr)
        return 2


class FractusApp:
    def __init__(self, root: tk.Tk, scene: Scene, args: argparse.Namespace):
        self.root = root
        self.root.title("Fractus v2 // Live Geometry Studio")
        # Anchor position to avoid overlap with music windows. Prefer --geometry or --x --y.
        geometry = getattr(args, "geometry", None)
        x = getattr(args, "x", None)
        y = getattr(args, "y", None)
        if geometry:
            self.root.geometry(geometry)
        elif x is not None and y is not None:
            w = getattr(args, "width", None) or 900
            h = getattr(args, "height", None) or 650
            self.root.geometry(f"{w}x{h}+{x}+{y}")
        else:
            self.root.geometry("1260x790")
        self.root.minsize(980, 650)
        self.root.configure(bg=BG)
        self.root.protocol("WM_DELETE_WINDOW", self.close)

        self.scene = validate_scene(scene)
        self.current_result: RenderResult | None = None
        self.current_result_scene: Scene | None = None
        self.photo: ImageTk.PhotoImage | None = None
        self.render_queue: queue.Queue[tuple[Any, ...]] = queue.Queue()
        self.render_requests: queue.Queue[tuple[Any, ...]] = queue.Queue(maxsize=1)
        self.worker_stop = threading.Event()
        self.render_generation = 0
        self.cancel_event: threading.Event | None = None
        self.active_command: CommandEnvelope | None = None
        self.last_sequence = -1
        self.seen_command_ids: set[str] = set()
        self.command_token: str | None = None
        self.startup_command: CommandEnvelope | None = None
        self.startup_command_error: Exception | None = None
        try:
            startup_token, startup_command = read_changed_command(None, COMMAND_PATH)
            self.command_token = startup_token
            if (
                startup_command is not None
                and startup_command.schema_version == 2
                and not command_has_terminal_status(startup_command.command_id)
            ):
                self.startup_command = startup_command
        except Exception as exc:
            self.command_token = command_token(COMMAND_PATH)
            self.startup_command_error = exc
        self.mutation_index = 0
        self.playing = bool(getattr(args, "play", False) and self.scene.animations)
        self.frame_index = max(0, int(args.frame))
        self.fps = float(args.fps)
        self.animation_started_at: float | None = (
            time.monotonic() - self.frame_index / self.fps if self.playing else None
        )
        self.auto_save_path = Path(args.output or args.save) if (args.output or args.save) else None
        self.close_after_auto_save = bool(args.save)
        self.display_rect = (0, 0, 1, 1)
        self._last_progress_sent = 0.0
        self.frame_cache = {}
        self._closed = False

        self._build_ui()
        self.render_worker = threading.Thread(
            target=self._render_worker_loop,
            daemon=True,
            name="FractusLatestRenderWorker",
        )
        self.render_worker.start()
        self._sync_controls_from_scene()
        self._set_editor(format_live_code(self.scene))
        self.root.bind("<Control-Return>", self.compile_live_code)
        self.root.bind("<Control-s>", self.save_dialog)
        self.root.after(15, self._poll_render_queue)
        self.root.after(250, self._poll_command_file)
        if self.startup_command_error is not None:
            self.root.after(120, self._reject_startup_command)
        elif self.startup_command is not None:
            self.root.after(120, lambda: self._apply_command(self.startup_command))
        else:
            self.root.after(150, lambda: self.submit_render(self.scene))

    def _reject_startup_command(self) -> None:
        error = self.startup_command_error
        if error is None:
            return
        self.status_var.set(f"Startup command rejected: {error}")
        self._publish_status(
            make_status("invalid-command", "rejected", detail=str(error))
        )

    def _build_ui(self) -> None:
        style = ttk.Style()
        style.theme_use("default")
        style.configure("TFrame", background=BG)
        style.configure("Panel.TFrame", background=PANEL)
        style.configure("TLabel", background=PANEL, foreground=TEXT, font=("Consolas", 9))
        style.configure("TButton", background=FIELD, foreground=TEXT, font=("Consolas", 9, "bold"))
        style.configure("TNotebook", background=BG, borderwidth=0)
        style.configure("TNotebook.Tab", background=FIELD, foreground=TEXT, padding=(10, 5))
        style.configure("TCombobox", fieldbackground=FIELD, background=FIELD, foreground=TEXT)
        style.configure("Cyber.Horizontal.TProgressbar", troughcolor=FIELD, background=MAGENTA)

        paned = ttk.Panedwindow(self.root, orient="horizontal")
        paned.pack(fill="both", expand=True, padx=10, pady=10)

        preview_frame = tk.Frame(paned, bg=BG, highlightbackground=VIOLET, highlightthickness=1)
        panel = ttk.Frame(paned, style="Panel.TFrame", width=430)
        paned.add(preview_frame, weight=3)
        paned.add(panel, weight=2)

        header = tk.Frame(preview_frame, bg=BG)
        header.pack(fill="x", padx=10, pady=(8, 4))
        tk.Label(
            header,
            text="F R A C T U S  v2",
            bg=BG,
            fg=MAGENTA,
            font=("Consolas", 18, "bold"),
        ).pack(side="left")
        tk.Label(
            header,
            text="deterministic live geometry",
            bg=BG,
            fg=CYAN,
            font=("Consolas", 9, "italic"),
        ).pack(side="right")

        self.canvas = tk.Canvas(preview_frame, bg=BG, highlightthickness=0, cursor="crosshair")
        self.canvas.pack(fill="both", expand=True, padx=8, pady=4)
        self.canvas_image_id = self.canvas.create_image(0, 0, anchor="nw")
        self.canvas.bind("<Configure>", self._on_canvas_resize)
        self.canvas.bind("<Button-1>", lambda event: self._zoom(event, 0.5))
        self.canvas.bind("<Button-3>", lambda event: self._zoom(event, 2.0))

        footer = tk.Frame(preview_frame, bg=BG)
        footer.pack(fill="x", padx=10, pady=(4, 8))
        self.progress = ttk.Progressbar(
            footer, style="Cyber.Horizontal.TProgressbar", mode="determinate", maximum=100
        )
        self.progress.pack(fill="x")
        self.status_var = tk.StringVar(value="System ready")
        tk.Label(
            footer,
            textvariable=self.status_var,
            bg=BG,
            fg=CYAN,
            anchor="w",
            font=("Consolas", 8),
        ).pack(fill="x", pady=(3, 0))

        notebook = ttk.Notebook(panel)
        notebook.pack(fill="both", expand=True, padx=8, pady=8)
        live_tab = ttk.Frame(notebook, style="Panel.TFrame")
        quick_tab = ttk.Frame(notebook, style="Panel.TFrame")
        caps_tab = ttk.Frame(notebook, style="Panel.TFrame")
        notebook.add(live_tab, text="LIVE CODE")
        notebook.add(quick_tab, text="QUICK")
        notebook.add(caps_tab, text="FAMILIES")

        tk.Label(
            live_tab,
            text="SAFE SCENE CODE  ·  CTRL+ENTER TO COMPILE",
            bg=PANEL,
            fg=CYAN,
            font=("Consolas", 9, "bold"),
        ).pack(anchor="w", padx=8, pady=(10, 5))
        editor_frame = tk.Frame(live_tab, bg=FIELD)
        editor_frame.pack(fill="both", expand=True, padx=8)
        scrollbar = tk.Scrollbar(editor_frame)
        scrollbar.pack(side="right", fill="y")
        self.editor = tk.Text(
            editor_frame,
            wrap="none",
            undo=True,
            bg="#090412",
            fg=TEXT,
            insertbackground=CYAN,
            selectbackground="#4b1675",
            font=("Consolas", 9),
            relief="flat",
            padx=8,
            pady=8,
            yscrollcommand=scrollbar.set,
        )
        self.editor.pack(fill="both", expand=True)
        scrollbar.configure(command=self.editor.yview)
        self.diagnostic_var = tk.StringVar(value="Bounded DSL: no eval, imports, shell, or network")
        tk.Label(
            live_tab,
            textvariable=self.diagnostic_var,
            wraplength=390,
            justify="left",
            bg=PANEL,
            fg=MUTED,
            font=("Consolas", 8),
        ).pack(fill="x", padx=8, pady=5)
        live_buttons = tk.Frame(live_tab, bg=PANEL)
        live_buttons.pack(fill="x", padx=8, pady=(0, 9))
        self.compile_button = tk.Button(
            live_buttons,
            text="[ COMPILE + RENDER ]",
            command=self.compile_live_code,
            bg=FIELD,
            fg=CYAN,
            activebackground="#3b1958",
            activeforeground="white",
            relief="flat",
            font=("Consolas", 9, "bold"),
        )
        self.compile_button.pack(fill="x", pady=2)
        row = tk.Frame(live_buttons, bg=PANEL)
        row.pack(fill="x")
        self.play_button = tk.Button(
            row,
            text="PAUSE" if self.playing else "PLAY",
            command=self.toggle_play,
            bg=FIELD,
            fg=TEXT,
            relief="flat",
        )
        self.play_button.pack(side="left", fill="x", expand=True, padx=(0, 2))
        tk.Button(row, text="MUTATE", command=self.mutate, bg=FIELD, fg=MAGENTA, relief="flat").pack(side="left", fill="x", expand=True, padx=2)
        tk.Button(row, text="ABORT", command=self.abort_render, bg=FIELD, fg="#ff5c5c", relief="flat").pack(side="left", fill="x", expand=True, padx=(2, 0))
        row2 = tk.Frame(live_buttons, bg=PANEL)
        row2.pack(fill="x", pady=(3, 0))
        tk.Button(row2, text="RESET VIEW", command=self.reset_view, bg=FIELD, fg=TEXT, relief="flat").pack(side="left", fill="x", expand=True, padx=(0, 2))
        tk.Button(row2, text="SAVE PNG", command=self.save_dialog, bg=FIELD, fg=CYAN, relief="flat").pack(side="left", fill="x", expand=True, padx=(2, 0))
        tk.Button(row2, text="SAVE ANIM GIF", command=self.save_anim_gif, bg=FIELD, fg="#39ff14", relief="flat").pack(side="left", fill="x", expand=True, padx=(2, 0))

        self._build_quick_tab(quick_tab)
        self._build_capabilities_tab(caps_tab)

    def _build_quick_tab(self, tab: ttk.Frame) -> None:
        container = tk.Frame(tab, bg=PANEL)
        container.pack(fill="both", expand=True, padx=12, pady=12)

        def label(text: str) -> None:
            tk.Label(container, text=text, bg=PANEL, fg=TEXT, font=("Consolas", 9, "bold")).pack(anchor="w", pady=(7, 2))

        label("FAMILY")
        self.family_var = tk.StringVar()
        self.family_combo = ttk.Combobox(container, textvariable=self.family_var, values=sorted(FAMILY_SPECS), state="readonly")
        self.family_combo.pack(fill="x")
        label("PALETTE")
        self.palette_var = tk.StringVar()
        self.palette_combo = ttk.Combobox(container, textvariable=self.palette_var, values=sorted(PALETTES), state="normal")
        self.palette_combo.pack(fill="x")
        label("LEGACY DETAIL  ·  20—800")
        self.detail_var = tk.IntVar(value=180)
        tk.Scale(
            container,
            variable=self.detail_var,
            from_=20,
            to=800,
            orient="horizontal",
            bg=PANEL,
            fg=TEXT,
            troughcolor=FIELD,
            activebackground=VIOLET,
            highlightthickness=0,
        ).pack(fill="x")
        label("DETERMINISTIC SEED")
        self.seed_var = tk.StringVar()
        tk.Entry(container, textvariable=self.seed_var, bg=FIELD, fg=CYAN, insertbackground=CYAN, relief="flat").pack(fill="x")
        constants = tk.Frame(container, bg=PANEL)
        constants.pack(fill="x", pady=8)
        tk.Label(constants, text="C REAL", bg=PANEL, fg=TEXT).grid(row=0, column=0, sticky="w")
        tk.Label(constants, text="C IMAG", bg=PANEL, fg=TEXT).grid(row=0, column=1, sticky="w")
        self.c_real_var = tk.StringVar(value="-0.7")
        self.c_imag_var = tk.StringVar(value="0.27015")
        tk.Entry(constants, textvariable=self.c_real_var, bg=FIELD, fg=CYAN, insertbackground=CYAN, relief="flat").grid(row=1, column=0, sticky="ew", padx=(0, 3))
        tk.Entry(constants, textvariable=self.c_imag_var, bg=FIELD, fg=CYAN, insertbackground=CYAN, relief="flat").grid(row=1, column=1, sticky="ew", padx=(3, 0))
        constants.columnconfigure(0, weight=1)
        constants.columnconfigure(1, weight=1)
        tk.Button(
            container,
            text="[ APPLY TO FIRST LAYER ]",
            command=self.apply_quick_controls,
            bg=FIELD,
            fg=CYAN,
            activebackground="#3b1958",
            relief="flat",
            font=("Consolas", 9, "bold"),
        ).pack(fill="x", pady=(12, 5))
        tk.Label(
            container,
            text="Quick controls migrate the original CLI values onto layer 0 while preserving other layers, background, name, and compatible animations. Use Live Code for family-specific parameters.",
            bg=PANEL,
            fg=MUTED,
            wraplength=360,
            justify="left",
            font=("Consolas", 8),
        ).pack(fill="x", pady=8)

    def _build_capabilities_tab(self, tab: ttk.Frame) -> None:
        text = tk.Text(tab, bg="#090412", fg=TEXT, font=("Consolas", 9), relief="flat", padx=10, pady=10, wrap="word")
        text.pack(fill="both", expand=True, padx=8, pady=8)
        for category, families in families_by_category().items():
            text.insert("end", category.upper() + "\n")
            for family in families:
                spec = FAMILY_SPECS[family]
                text.insert("end", f"  {family}\n    {', '.join(spec.params)}\n")
            text.insert("end", "\n")
        text.configure(state="disabled")

    def _set_editor(self, source: str) -> None:
        self.editor.delete("1.0", "end")
        self.editor.insert("1.0", source)

    def _sync_controls_from_scene(self) -> None:
        first = self.scene.layers[0]
        self.family_var.set(first.family)
        self.palette_var.set(self.scene.palette)
        self.seed_var.set(str(self.scene.seed))
        params = first.params
        self.detail_var.set(int(params.get("iterations", params.get("density", 40) * 5)))
        self.c_real_var.set(str(params.get("c_real", params.get("phase", params.get("twist", -0.7)))))
        self.c_imag_var.set(str(params.get("c_imag", params.get("warp", 0.27015))))

    def compile_live_code(self, event: tk.Event | None = None) -> str | None:
        try:
            scene = parse_live_code(self.editor.get("1.0", "end-1c"))
        except SceneError as exc:
            self.diagnostic_var.set(str(exc))
            self.status_var.set(f"Compile rejected: {exc}")
            return "break" if event else None
        self.frame_cache.clear()
        self.scene = scene
        self._sync_controls_from_scene()
        self.diagnostic_var.set(f"Compiled {len(scene.layers)} layer(s) · recipe {scene.recipe_hash()[:12]}")
        self.submit_render(scene)
        return "break" if event else None

    def apply_quick_controls(self) -> None:
        try:
            migrated = legacy_scene(
                family=self.family_var.get(),
                iterations=int(self.detail_var.get()),
                palette=self.palette_var.get(),
                c_real=float(self.c_real_var.get()),
                c_imag=float(self.c_imag_var.get()),
                seed=int(self.seed_var.get()),
                width=self.scene.width,
                height=self.scene.height,
            )
            scene, removed_animations = merge_legacy_first_layer(self.scene, migrated)
        except (SceneError, ValueError) as exc:
            self.status_var.set(f"Quick controls rejected: {exc}")
            return
        self.frame_cache.clear()
        self.scene = scene
        self._set_editor(format_live_code(scene))
        suffix = (
            f"; removed {removed_animations} incompatible layer-0 animation(s)"
            if removed_animations
            else ""
        )
        self.diagnostic_var.set(
            "Quick controls updated layer 0 without discarding the composition" + suffix
        )
        self.submit_render(scene)

    def mutate(self) -> None:
        try:
            self.scene = mutate_scene(self.scene, self.mutation_index)
            self.mutation_index += 1
        except SceneError as exc:
            self.status_var.set(f"Mutation rejected: {exc}")
            return
        self.frame_cache.clear()
        self._set_editor(format_live_code(self.scene))
        self._sync_controls_from_scene()
        self.diagnostic_var.set(f"Deterministic variation {self.mutation_index} · seed {self.scene.seed}")
        self.submit_render(self.scene)

    def reset_view(self) -> None:
        first = self.scene.layers[0]
        defaults = default_layer(first.family)
        self.scene = validate_scene(replace(self.scene, layers=(replace(first, params=defaults.params), *self.scene.layers[1:])))
        self._set_editor(format_live_code(self.scene))
        self._sync_controls_from_scene()
        self.submit_render(self.scene)

    def _zoom(self, event: tk.Event, factor: float) -> None:
        first = self.scene.layers[0]
        spec = FAMILY_SPECS[first.family]
        if spec.renderer not in {"escape", "newton"}:
            self.status_var.set("Zoom navigation applies to complex fractal families; mutate this geometry instead")
            return
        left, top, right, bottom = self.display_rect
        if not (left <= event.x <= right and top <= event.y <= bottom):
            return
        nx = (event.x - left) / max(right - left, 1)
        ny = (event.y - top) / max(bottom - top, 1)
        params = dict(first.params)
        scale = float(params["scale"])
        x_scale = scale * self.scene.width / self.scene.height
        params["center_x"] = float(params["center_x"]) + (nx - 0.5) * x_scale
        params["center_y"] = float(params["center_y"]) + (0.5 - ny) * scale
        params["scale"] = max(1e-7, min(10.0, scale * factor))
        self.scene = validate_scene(replace(self.scene, layers=(replace(first, params=params), *self.scene.layers[1:])))
        self._set_editor(format_live_code(self.scene))
        self.submit_render(self.scene)

    def abort_render(self) -> None:
        if self.cancel_event is not None:
            self.cancel_event.set()
        while True:
            try:
                pending = self.render_requests.get_nowait()
                pending[3].set()
            except queue.Empty:
                break
        self.render_generation += 1
        if self.active_command is not None:
            self._publish_status(
                make_status(
                    self.active_command.command_id,
                    "cancelled",
                    sequence=self.active_command.sequence,
                    source=self.active_command.source,
                    detail="render cancelled by the operator",
                )
            )
            self.active_command = None
        self.status_var.set("Render cancelled")

    def _publish_status(self, payload: dict[str, Any]) -> None:
        try:
            write_status(payload)
        except OSError as exc:
            self.status_var.set(f"Status acknowledgement failed: {exc}")

    def submit_render(self, scene: Scene, command: CommandEnvelope | None = None) -> None:
        scene = validate_scene(scene)

        # Check cache first
        effective_scene = scene_for_frame(scene, self.frame_index, self.fps)
        recipe_hash = effective_scene.recipe_hash()
        if recipe_hash in self.frame_cache:
            result = self.frame_cache[recipe_hash]
            self.current_result = result
            self._display_result()
            self.progress["value"] = 100
            self.status_var.set(f"Loaded frame {self.frame_index} from cache")
            
            if command is not None:
                self._publish_status(
                    make_status(
                        command.command_id,
                        "completed",
                        sequence=command.sequence,
                        source=command.source,
                        result=result,
                    )
                )
                self.active_command = None

            if self.playing:
                self.frame_index += 1
                generation = self.render_generation
                delay = max(1, int(1000 / self.fps))
                self.root.after(
                    delay,
                    lambda generation=generation, scene=scene: self._continue_animation(
                        generation, scene
                    ),
                )
            return

        if self.cancel_event is not None:
            self.cancel_event.set()
            if self.active_command is not None:
                self._publish_status(
                    make_status(
                        self.active_command.command_id,
                        "superseded",
                        sequence=self.active_command.sequence,
                        source=self.active_command.source,
                        detail="a newer scene replaced this render",
                    )
                )
        self.render_generation += 1
        generation = self.render_generation
        cancel_event = threading.Event()
        frame_index = self.frame_index
        fps = self.fps
        self.cancel_event = cancel_event
        self.active_command = command
        self.scene = scene
        self.progress["value"] = 0
        self.status_var.set(f"Rendering {scene.name}…")
        if command is not None:
            self._publish_status(make_status(command.command_id, "accepted", sequence=command.sequence, source=command.source, detail=f"accepted {len(scene.layers)} validated layer(s)"))
            self._publish_status(make_status(command.command_id, "rendering", sequence=command.sequence, source=command.source, detail="render worker started"))

        request = (generation, scene, command, cancel_event, frame_index, fps)
        while True:
            try:
                stale = self.render_requests.get_nowait()
                stale[3].set()
            except queue.Empty:
                break
        self.render_requests.put_nowait(request)

    def _continue_animation(self, generation: int, scene: Scene) -> None:
        """Advance only the animation frame that scheduled this callback.

        Tk callbacks are not cancelled automatically when a mailbox command
        installs a newer scene.  Without these ownership checks, a stale
        callback can run after the external command was accepted and call
        ``submit_render(..., command=None)``, falsely superseding that command.
        """
        if (
            self._closed
            or not self.playing
            or generation != self.render_generation
            or self.active_command is not None
            or self.scene is not scene
        ):
            return
        self.submit_render(scene)

    def _render_worker_loop(self) -> None:
        while not self.worker_stop.is_set():
            try:
                generation, scene, command, cancel_event, frame_index, fps = (
                    self.render_requests.get(timeout=0.1)
                )
            except queue.Empty:
                continue
            last_progress = 0.0

            def report(value: float, stage: str) -> None:
                nonlocal last_progress
                now = time.monotonic()
                if value >= 1.0 or now - last_progress >= 0.06:
                    last_progress = now
                    self.render_queue.put(("progress", generation, value, stage))

            if cancel_event.is_set():
                self.render_queue.put(("cancelled", generation, command))
                continue
            try:
                result = render_scene(
                    scene,
                    frame_index=frame_index,
                    fps=fps,
                    progress=report,
                    cancel=cancel_event.is_set,
                )
                self.render_queue.put(("done", generation, result, scene, command))
            except RenderCancelled:
                self.render_queue.put(("cancelled", generation, command))
            except Exception as exc:
                self.render_queue.put(("error", generation, exc, command))

    def _poll_render_queue(self) -> None:
        if self._closed:
            return
        try:
            while True:
                event = self.render_queue.get_nowait()
                kind, generation = event[0], event[1]
                if generation != self.render_generation:
                    continue
                if kind == "progress":
                    _, _, value, stage = event
                    self.progress["value"] = value * 100
                    self.status_var.set(stage)
                elif kind == "done":
                    _, _, result, scene, command = event
                    self.current_result = result
                    self.current_result_scene = scene
                    self.progress["value"] = 100
                    self._display_result()
                    self.status_var.set(
                        f"Completed in {result.duration_ms} ms · render {result.render_hash[:12]}"
                    )
                    saved: Path | None = None
                    try:
                        if command is not None and command.persist:
                            saved = save_render_result(
                                result,
                                scene,
                                command.output_path
                                or OUTPUT_DIR / f"{command_file_stem(command.command_id)}.png",
                            )
                        if self.auto_save_path is not None:
                            saved = save_render_result(result, scene, self.auto_save_path)
                            self.status_var.set(f"Saved {saved}")
                            self.auto_save_path = None
                            if self.close_after_auto_save:
                                self.root.after(250, self.close)

                        # Autonomous animation: if the scene has animations and was requested by an external command,
                        # auto-render and save a GIF alongside the PNG.
                        if command is not None and scene.animations:
                            try:
                                gif_path = OUTPUT_DIR / f"{command_file_stem(command.command_id)}_anim.gif"
                                gif = render_animated_gif(scene, num_frames=24, fps=12, output_path=gif_path)
                                self.status_var.set(f"Auto-saved animated GIF: {gif}")
                                # publish to court via status
                                anim_payload = make_status(
                                    command.command_id,
                                    "completed",
                                    source="animation",
                                    output_path=gif,
                                    detail="animated via autonomous render",
                                )
                                atomic_write_json(STATUS_PATH.with_suffix('.anim.json'), anim_payload)
                            except Exception as anim_exc:
                                self.status_var.set(f"Auto-anim GIF failed: {anim_exc}")
                    except Exception as exc:
                        self.status_var.set(f"Save failed: {exc}")
                        if command is not None:
                            self._publish_status(make_status(command.command_id, "rejected", sequence=command.sequence, source=command.source, detail=f"render completed but save failed: {exc}"))
                        self.active_command = None
                        continue
                    if command is not None:
                        self._publish_status(make_status(command.command_id, "completed", sequence=command.sequence, source=command.source, result=result, output_path=saved))
                    self.active_command = None
                    # Save to cache
                    effective_scene = scene_for_frame(scene, result.frame_index, self.fps)
                    self.frame_cache[effective_scene.recipe_hash()] = result
                    if len(self.frame_cache) > 600:
                        self.frame_cache.pop(next(iter(self.frame_cache)))

                    if self.playing:
                        self.frame_index += 1
                        self.root.after(
                            15,
                            lambda generation=generation, scene=scene: self._continue_animation(
                                generation, scene
                            ),
                        )
                elif kind == "cancelled":
                    _, _, command = event
                    self.status_var.set("Render cancelled")
                    if command is not None:
                        self._publish_status(
                            make_status(
                                command.command_id,
                                "cancelled",
                                sequence=command.sequence,
                                source=command.source,
                                detail="render was cancelled before completion",
                            )
                        )
                    self.active_command = None
                elif kind == "error":
                    _, _, error, command = event
                    self.status_var.set(f"Render rejected: {error}")
                    self.diagnostic_var.set(str(error))
                    if command is not None:
                        self._publish_status(make_status(command.command_id, "rejected", sequence=command.sequence, source=command.source, detail=str(error)))
                    self.active_command = None
        except queue.Empty:
            pass
        self.root.after(15, self._poll_render_queue)

    def _on_canvas_resize(self, event: tk.Event | None = None) -> None:
        if self.current_result is not None:
            self._display_result()

    def _display_result(self) -> None:
        if self.current_result is None:
            return
        width = max(1, self.canvas.winfo_width())
        height = max(1, self.canvas.winfo_height())
        image = self.current_result.image.copy()
        image.thumbnail((width, height), Image.Resampling.BILINEAR)
        left = (width - image.width) // 2
        top = (height - image.height) // 2
        self.display_rect = (left, top, left + image.width, top + image.height)
        self.photo = ImageTk.PhotoImage(image)
        self.canvas.itemconfigure(self.canvas_image_id, image=self.photo)
        self.canvas.coords(self.canvas_image_id, left, top)

    def save_dialog(self, event: tk.Event | None = None) -> str | None:
        if self.current_result is None or self.current_result_scene is None:
            self.status_var.set("Nothing has completed rendering yet")
            return "break" if event else None
        filename = filedialog.asksaveasfilename(
            title="Save Fractus artwork",
            initialdir=str(OUTPUT_DIR),
            initialfile=f"{self.current_result_scene.layers[0].family}_{self.current_result.render_hash[:8]}.png",
            defaultextension=".png",
            filetypes=(("PNG image", "*.png"), ("JPEG image", "*.jpg"), ("WebP image", "*.webp")),
        )
        if not filename:
            return "break" if event else None
        try:
            saved = save_render_result(self.current_result, self.current_result_scene, filename)
        except Exception as exc:
            self.status_var.set(f"Save failed: {exc}")
            messagebox.showerror("Fractus Save Failed", str(exc))
        else:
            self.status_var.set(f"Saved {saved}")
            messagebox.showinfo("Fractus", f"Artwork and manifest saved:\n{saved}")
        return "break" if event else None

    def save_anim_gif(self, event: tk.Event | None = None) -> str | None:
        if not self.scene.animations:
            self.status_var.set("Current scene has no animations. Add `animate ...` lines.")
            return "break" if event else None
        try:
            path = render_animated_gif(
                self.scene,
                num_frames=30,
                fps=12,
                cancel=lambda: self.cancel_event.is_set() if self.cancel_event else False,
            )
            self.status_var.set(f"Saved animated GIF: {path}")
            messagebox.showinfo("Fractus", f"Animated GIF saved:\n{path}")
        except Exception as e:
            self.status_var.set(f"Anim GIF failed: {e}")
        return "break" if event else None

    def toggle_play(self) -> None:
        if not self.scene.animations:
            self.status_var.set("Add an `animate 0.parameter from=… to=…` line before playing")
            return
        self.playing = not self.playing
        self.play_button.configure(text="PAUSE" if self.playing else "PLAY")
        if self.playing:
            self.animation_started_at = time.monotonic() - self.frame_index / self.fps
            self.submit_render(self.scene)
        elif self.cancel_event is not None:
            self.cancel_event.set()

    def _poll_command_file(self) -> None:
        if self._closed:
            return
        try:
            token, command = read_changed_command(self.command_token, COMMAND_PATH)
            self.command_token = token
        except Exception as exc:
            self.command_token = command_token(COMMAND_PATH)
            self.status_var.set(f"External command rejected: {exc}")
            try:
                self._publish_status(make_status("invalid-command", "rejected", detail=str(exc)))
            except OSError:
                pass
        else:
            if command is not None:
                self._apply_command(command)
        self.root.after(250, self._poll_command_file)

    def _apply_command(self, command: CommandEnvelope) -> None:
        if command.schema_version == 2:
            if command.command_id in self.seen_command_ids:
                self._publish_status(make_status(command.command_id, "superseded", sequence=command.sequence, source=command.source, detail="duplicate command_id"))
                return
            if command.sequence and command.sequence <= self.last_sequence:
                self._publish_status(make_status(command.command_id, "superseded", sequence=command.sequence, source=command.source, detail="stale sequence"))
                return
            self.seen_command_ids.add(command.command_id)
            if command.sequence:
                self.last_sequence = command.sequence
        if command.action == "close":
            self._publish_status(make_status(command.command_id, "closing", sequence=command.sequence, source=command.source))
            self.root.after(50, self.close)
            return
        if command.action == "ping":
            self._publish_status(make_status(command.command_id, "pong", sequence=command.sequence, source=command.source, detail="Fractus v2 is responsive"))
            return
        if command.scene is None:
            self._publish_status(make_status(command.command_id, "rejected", sequence=command.sequence, source=command.source, detail="apply command had no scene"))
            return
        self.scene = command.scene
        self.playing = bool(self.scene.animations)
        self.animation_started_at = time.monotonic() - self.frame_index / self.fps if self.playing else None
        self.play_button.configure(text="PAUSE" if self.playing else "PLAY")
        self._set_editor(format_live_code(self.scene))
        self._sync_controls_from_scene()
        self.diagnostic_var.set(f"External {command.source} command accepted · {command.command_id}")
        self.submit_render(self.scene, command)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self.playing = False
        self.worker_stop.set()
        if self.cancel_event is not None:
            self.cancel_event.set()
        if self.active_command is not None:
            try:
                self._publish_status(
                    make_status(
                        self.active_command.command_id,
                        "cancelled",
                        sequence=self.active_command.sequence,
                        source=self.active_command.source,
                        detail="Fractus closed before this render completed",
                    )
                )
            except OSError:
                pass
            self.active_command = None
        self.root.destroy()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Fractus v2 live geometry studio")
    parser.add_argument("--type", choices=sorted(FAMILY_SPECS), help="Legacy single-layer family")
    parser.add_argument("--iterations", type=int, help="Legacy detail value, consistently bounded to 20-800")
    parser.add_argument("--palette", help="Named palette or comma-separated #RRGGBB gradient")
    parser.add_argument("--c-real", type=float, help="Legacy real/phase/twist parameter")
    parser.add_argument("--c-imag", type=float, help="Legacy imaginary/warp parameter")
    parser.add_argument("--seed", type=int, help="Explicit deterministic seed")
    parser.add_argument("--width", type=int, default=600)
    parser.add_argument("--height", type=int, default=600)
    parser.add_argument("--x", type=int, default=None, help="Window x position to anchor (e.g. to avoid overlap with music)")
    parser.add_argument("--y", type=int, default=None, help="Window y position")
    parser.add_argument("--geometry", help="Full Tk geometry e.g. 900x650+50+50 to anchor position/size")
    parser.add_argument("--save", help="GUI auto-save path; closes only after a successful save")
    parser.add_argument("--output", help="Headless or GUI output path")
    parser.add_argument("--script", help="Path to a .fract live-code document")
    parser.add_argument("--code", help="Inline Fractus live code")
    parser.add_argument("--mutate", type=int, metavar="INDEX", help="Apply deterministic mutation INDEX")
    parser.add_argument("--frame", type=int, default=0, help="Deterministic animation frame")
    parser.add_argument("--fps", type=float, default=30.0, help="Animation frame rate used for parameter time")
    parser.add_argument("--play", action="store_true", help="Start playback immediately when the scene has animations")
    parser.add_argument("--headless", action="store_true", help="Render without opening tkinter")
    parser.add_argument("--validate", action="store_true", help="Validate and print canonical scene JSON")
    parser.add_argument("--status", help="Write headless status JSON atomically")
    parser.add_argument("--capabilities", action="store_true", help="Print the v2 registry manifest")
    parser.add_argument("--self-test", action="store_true", help="Render every registered family at smoke-test resolution")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.headless or args.validate or args.capabilities or args.self_test:
        return run_headless(args)
    try:
        scene = _scene_from_args(args)
    except Exception as exc:
        parser.error(str(exc))
    root = tk.Tk()
    FractusApp(root, scene, args)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

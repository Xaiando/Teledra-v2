"""Pure data model for the Fractus v2 geometry engine.

The model deliberately contains no tkinter objects and performs no file I/O.  It
is shared by the desktop UI, headless renderer, protocol adapter, and tests.
"""

from __future__ import annotations

from dataclasses import dataclass, field, replace
import hashlib
import json
import math
from typing import Any, Mapping


SCHEMA_VERSION = 2
MIN_CANVAS = 64
MAX_CANVAS = 2048
MAX_PIXELS = 4_194_304
MAX_LAYERS = 8
MAX_ANIMATIONS = 16
MAX_SOURCE_CHARS = 64_000


class SceneError(ValueError):
    """Raised when a scene or live-code document violates the v2 contract."""


@dataclass(frozen=True)
class Layer:
    family: str
    params: Mapping[str, Any] = field(default_factory=dict)
    alpha: float = 1.0
    blend: str = "normal"
    visible: bool = True


@dataclass(frozen=True)
class Animation:
    target: str
    start: float
    end: float
    seconds: float = 8.0
    easing: str = "sine"
    loop: bool = True
    pingpong: bool = False


@dataclass(frozen=True)
class Scene:
    width: int = 600
    height: int = 600
    seed: int = 1
    palette: str = "purple_haze"
    background: str | None = None
    layers: tuple[Layer, ...] = field(default_factory=tuple)
    animations: tuple[Animation, ...] = field(default_factory=tuple)
    name: str = "Untitled Geometry"
    version: int = SCHEMA_VERSION

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": self.version,
            "name": self.name,
            "canvas": {"width": self.width, "height": self.height},
            "seed": self.seed,
            "palette": self.palette,
            "background": self.background,
            "layers": [
                {
                    "family": layer.family,
                    "params": dict(sorted(layer.params.items())),
                    "alpha": layer.alpha,
                    "blend": layer.blend,
                    "visible": layer.visible,
                }
                for layer in self.layers
            ],
            "animations": [
                {
                    "target": animation.target,
                    "start": animation.start,
                    "end": animation.end,
                    "seconds": animation.seconds,
                    "easing": animation.easing,
                    "loop": animation.loop,
                    "pingpong": animation.pingpong,
                }
                for animation in self.animations
            ],
        }

    def canonical_json(self) -> str:
        return json.dumps(
            self.to_dict(),
            ensure_ascii=True,
            allow_nan=False,
            sort_keys=True,
            separators=(",", ":"),
        )

    def recipe_hash(self) -> str:
        return hashlib.sha256(self.canonical_json().encode("utf-8")).hexdigest()


def parse_color(value: str, field_name: str = "color") -> str:
    value = str(value).strip()
    if len(value) == 7 and value.startswith("#"):
        try:
            int(value[1:], 16)
        except ValueError as exc:
            raise SceneError(f"{field_name} must be a #RRGGBB color") from exc
        return value.lower()
    raise SceneError(f"{field_name} must be a #RRGGBB color")


def ensure_finite_number(value: Any, field_name: str) -> float:
    if isinstance(value, bool):
        raise SceneError(f"{field_name} must be numeric, not boolean")
    try:
        number = float(value)
    except (TypeError, ValueError) as exc:
        raise SceneError(f"{field_name} must be numeric") from exc
    if not math.isfinite(number):
        raise SceneError(f"{field_name} must be finite")
    return number


def validate_model_bounds(scene: Scene) -> None:
    if scene.version != SCHEMA_VERSION:
        raise SceneError(
            f"unsupported scene version {scene.version}; expected {SCHEMA_VERSION}"
        )
    if not isinstance(scene.width, int) or not isinstance(scene.height, int):
        raise SceneError("canvas width and height must be integers")
    if not (MIN_CANVAS <= scene.width <= MAX_CANVAS):
        raise SceneError(f"canvas width must be between {MIN_CANVAS} and {MAX_CANVAS}")
    if not (MIN_CANVAS <= scene.height <= MAX_CANVAS):
        raise SceneError(f"canvas height must be between {MIN_CANVAS} and {MAX_CANVAS}")
    if scene.width * scene.height > MAX_PIXELS:
        raise SceneError(f"canvas exceeds the {MAX_PIXELS:,}-pixel safety budget")
    if not isinstance(scene.seed, int) or isinstance(scene.seed, bool):
        raise SceneError("seed must be an integer")
    if not (-(2**63) <= scene.seed < 2**63):
        raise SceneError("seed must fit in a signed 64-bit integer")
    if not scene.layers:
        raise SceneError("a scene must contain at least one layer")
    if len(scene.layers) > MAX_LAYERS:
        raise SceneError(f"a scene may contain at most {MAX_LAYERS} layers")
    if len(scene.animations) > MAX_ANIMATIONS:
        raise SceneError(f"a scene may contain at most {MAX_ANIMATIONS} animations")
    if scene.background is not None:
        parse_color(scene.background, "background")
    if len(scene.name) > 120:
        raise SceneError("scene name must be at most 120 characters")
    for index, layer in enumerate(scene.layers):
        alpha = ensure_finite_number(layer.alpha, f"layer {index} alpha")
        if not 0.0 <= alpha <= 1.0:
            raise SceneError(f"layer {index} alpha must be between 0 and 1")
        if layer.blend not in {"normal", "screen", "multiply", "add"}:
            raise SceneError(
                f"layer {index} blend must be normal, screen, multiply, or add"
            )
    for animation in scene.animations:
        if not animation.target:
            raise SceneError("animation target cannot be empty")
        ensure_finite_number(animation.start, "animation start")
        ensure_finite_number(animation.end, "animation end")
        seconds = ensure_finite_number(animation.seconds, "animation seconds")
        if not 0.1 <= seconds <= 3_600.0:
            raise SceneError("animation seconds must be between 0.1 and 3600")
        if animation.easing not in {"linear", "sine", "smoothstep"}:
            raise SceneError("animation easing must be linear, sine, or smoothstep")


def _animation_progress(animation: Animation, frame_index: int, fps: float) -> float:
    fps = ensure_finite_number(fps, "fps")
    if fps <= 0 or fps > 240:
        raise SceneError("fps must be greater than 0 and at most 240")
    elapsed = max(0, frame_index) / fps
    raw = elapsed / animation.seconds
    if animation.loop:
        if animation.pingpong:
            raw = raw % 2.0
            raw = raw if raw <= 1.0 else 2.0 - raw
        else:
            raw %= 1.0
    else:
        raw = min(max(raw, 0.0), 1.0)
    if animation.easing == "sine":
        return 0.5 - 0.5 * math.cos(math.pi * raw)
    if animation.easing == "smoothstep":
        return raw * raw * (3.0 - 2.0 * raw)
    return raw


def scene_for_frame(scene: Scene, frame_index: int, fps: float = 30.0) -> Scene:
    """Return a deterministic scene snapshot for an animation frame."""

    if not scene.animations:
        return scene
    layers = [replace(layer, params=dict(layer.params)) for layer in scene.layers]
    for animation in scene.animations:
        parts = animation.target.split(".", 1)
        if len(parts) != 2 or not parts[0].isdigit():
            raise SceneError(
                f"animation target '{animation.target}' must be <layer-index>.<parameter>"
            )
        index = int(parts[0])
        parameter = parts[1]
        if not (0 <= index < len(layers)):
            raise SceneError(f"animation target layer {index} does not exist")
        progress = _animation_progress(animation, frame_index, fps)
        value = animation.start + (animation.end - animation.start) * progress
        params = dict(layers[index].params)
        params[parameter] = value
        layers[index] = replace(layers[index], params=params)
    return replace(scene, layers=tuple(layers))


def scene_from_dict(data: Mapping[str, Any]) -> Scene:
    """Deserialize the stable v2 dictionary form without registry validation."""

    try:
        allowed_root = {
            "version",
            "name",
            "canvas",
            "width",
            "height",
            "seed",
            "palette",
            "background",
            "layers",
            "animations",
        }
        unknown_root = sorted(set(data) - allowed_root)
        if unknown_root:
            raise SceneError(
                f"unsupported scene field(s): {', '.join(unknown_root)}"
            )
        canvas = data.get("canvas", {})
        if not isinstance(canvas, dict):
            raise SceneError("canvas must be an object")
        unknown_canvas = sorted(set(canvas) - {"width", "height"})
        if unknown_canvas:
            raise SceneError(
                f"unsupported canvas field(s): {', '.join(unknown_canvas)}"
            )
        layer_items = data.get("layers", [])
        animation_items = data.get("animations", [])
        if not isinstance(layer_items, list) or not all(
            isinstance(item, dict) for item in layer_items
        ):
            raise SceneError("layers must be an array of objects")
        if not isinstance(animation_items, list) or not all(
            isinstance(item, dict) for item in animation_items
        ):
            raise SceneError("animations must be an array of objects")
        layers_list = []
        for item in layer_items:
            unknown_layer = sorted(
                set(item) - {"family", "params", "alpha", "blend", "visible"}
            )
            if unknown_layer:
                raise SceneError(
                    f"unsupported layer field(s): {', '.join(unknown_layer)}"
                )
            visible = item.get("visible", True)
            if not isinstance(visible, bool):
                raise SceneError("layer visible must be true or false")
            alpha = item.get("alpha", 1.0)
            if isinstance(alpha, bool):
                raise SceneError("layer alpha must be numeric, not boolean")
            params = item.get("params", {})
            if not isinstance(params, dict):
                raise SceneError("layer params must be an object")
            layers_list.append(
                Layer(
                    family=str(item["family"]),
                    params=dict(params),
                    alpha=float(alpha),
                    blend=str(item.get("blend", "normal")),
                    visible=visible,
                )
            )
        layers = tuple(layers_list)
        animations_list = []
        for item in animation_items:
            unknown_animation = sorted(
                set(item)
                - {"target", "start", "end", "seconds", "easing", "loop", "pingpong"}
            )
            if unknown_animation:
                raise SceneError(
                    f"unsupported animation field(s): {', '.join(unknown_animation)}"
                )
            loop = item.get("loop", True)
            pingpong = item.get("pingpong", False)
            if not isinstance(loop, bool) or not isinstance(pingpong, bool):
                raise SceneError("animation loop and pingpong must be true or false")
            animations_list.append(
                Animation(
                    target=str(item["target"]),
                    start=float(item["start"]),
                    end=float(item["end"]),
                    seconds=float(item.get("seconds", 8.0)),
                    easing=str(item.get("easing", "sine")),
                    loop=loop,
                    pingpong=pingpong,
                )
            )
        animations = tuple(animations_list)
        seed_value = data.get("seed", 1)
        if not isinstance(seed_value, int) or isinstance(seed_value, bool):
            raise SceneError("seed must be an integer")
        version_value = data.get("version", SCHEMA_VERSION)
        if not isinstance(version_value, int) or isinstance(version_value, bool):
            raise SceneError("version must be an integer")
        width_value = canvas.get("width", data.get("width", 600))
        height_value = canvas.get("height", data.get("height", 600))
        if not isinstance(width_value, int) or isinstance(width_value, bool):
            raise SceneError("canvas width must be an integer")
        if not isinstance(height_value, int) or isinstance(height_value, bool):
            raise SceneError("canvas height must be an integer")
        name_value = data.get("name", "Untitled Geometry")
        palette_value = data.get("palette", "purple_haze")
        background_value = data.get("background")
        if not isinstance(name_value, str):
            raise SceneError("name must be text")
        if not isinstance(palette_value, str):
            raise SceneError("palette must be text")
        if background_value is not None and not isinstance(background_value, str):
            raise SceneError("background must be text or null")
        scene = Scene(
            width=width_value,
            height=height_value,
            seed=seed_value,
            palette=palette_value,
            background=(
                background_value if background_value is not None else None
            ),
            layers=layers,
            animations=animations,
            name=name_value,
            version=version_value,
        )
    except SceneError:
        raise
    except (KeyError, TypeError, ValueError, AttributeError) as exc:
        raise SceneError(f"invalid scene object: {exc}") from exc
    validate_model_bounds(scene)
    return scene

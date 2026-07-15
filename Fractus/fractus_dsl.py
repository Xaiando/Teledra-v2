"""Bounded, declarative live-code language for Fractus scenes.

The language intentionally has no expressions, imports, loops, filesystem
access, or evaluation hooks.  Its power comes from composing registered layers
and animating typed numeric parameters.
"""

from __future__ import annotations

from dataclasses import replace
import hashlib
import random
import re
import shlex
from typing import Any

from fractus_model import (
    MAX_SOURCE_CHARS,
    Animation,
    Layer,
    Scene,
    SceneError,
    ensure_finite_number,
)
from fractus_registry import (
    FAMILY_SPECS,
    PALETTES,
    default_layer,
    normalize_family,
    normalize_palette,
    validate_scene,
)


def _strip_comment(line: str) -> str:
    """Strip // only outside quoted strings, preserving URLs and names."""

    quote: str | None = None
    escaped = False
    index = 0
    while index < len(line):
        character = line[index]
        if escaped:
            escaped = False
        elif character == "\\":
            escaped = True
        elif quote is not None:
            if character == quote:
                quote = None
        elif character in {"'", '"'}:
            quote = character
        elif character == "/" and index + 1 < len(line) and line[index + 1] == "/":
            return line[:index]
        index += 1
    return line


def _tokens(line: str, line_number: int) -> list[str]:
    # `//` is the comment marker so #RRGGBB remains a legal token.
    line = _strip_comment(line)
    lexer = shlex.shlex(line, posix=True)
    lexer.whitespace_split = True
    lexer.commenters = ""
    try:
        return list(lexer)
    except ValueError as exc:
        raise SceneError(f"line {line_number}: {exc}") from exc


def _scalar(value: str) -> Any:
    lowered = value.lower()
    if lowered in {"true", "yes", "on"}:
        return True
    if lowered in {"false", "no", "off"}:
        return False
    try:
        if not any(character in value.lower() for character in (".", "e")):
            return int(value, 10)
        return float(value)
    except ValueError:
        return value


def _assignments(tokens: list[str], line_number: int) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for token in tokens:
        if "=" not in token:
            raise SceneError(
                f"line {line_number}: expected key=value, received '{token}'"
            )
        key, value = token.split("=", 1)
        key = key.strip().lower().replace("-", "_")
        if not key or not value:
            raise SceneError(f"line {line_number}: invalid assignment '{token}'")
        if key in result:
            raise SceneError(f"line {line_number}: duplicate assignment '{key}'")
        result[key] = _scalar(value)
    return result


def parse_live_code(source: str) -> Scene:
    if not isinstance(source, str):
        raise SceneError("live code must be text")
    if len(source) > MAX_SOURCE_CHARS:
        raise SceneError(
            f"live code exceeds the {MAX_SOURCE_CHARS:,}-character safety budget"
        )

    width = 600
    height = 600
    seed = 1
    palette = "purple_haze"
    background: str | None = None
    name = "Live Geometry"
    version = 2
    layers: list[Layer] = []
    animations: list[Animation] = []

    for line_number, line in enumerate(source.splitlines(), 1):
        parts = _tokens(line.strip(), line_number)
        if not parts:
            continue
        statement = parts[0].lower().replace("-", "_")
        args = parts[1:]

        if statement == "version":
            if len(args) != 1:
                raise SceneError(f"line {line_number}: version expects one integer")
            try:
                version = int(args[0])
            except ValueError as exc:
                raise SceneError(f"line {line_number}: version must be an integer") from exc
        elif statement == "name":
            if not args:
                raise SceneError(f"line {line_number}: name cannot be empty")
            name = " ".join(args)
        elif statement == "canvas":
            if len(args) != 2:
                raise SceneError(f"line {line_number}: canvas expects WIDTH HEIGHT")
            try:
                width, height = int(args[0]), int(args[1])
            except ValueError as exc:
                raise SceneError(
                    f"line {line_number}: canvas dimensions must be integers"
                ) from exc
        elif statement == "seed":
            if len(args) != 1:
                raise SceneError(f"line {line_number}: seed expects one integer")
            try:
                seed = int(args[0], 10)
            except ValueError as exc:
                raise SceneError(f"line {line_number}: seed must be an integer") from exc
        elif statement == "palette":
            if not args:
                raise SceneError(f"line {line_number}: palette cannot be empty")
            if args[0].startswith("#"):
                if not all(item.startswith("#") for item in args):
                    raise SceneError(
                        f"line {line_number}: custom palettes must contain only #RRGGBB stops"
                    )
                palette = normalize_palette(",".join(args))
            else:
                if len(args) != 1:
                    raise SceneError(
                        f"line {line_number}: named palette expects exactly one value"
                    )
                palette = normalize_palette(args[0])
        elif statement == "background":
            if len(args) != 1:
                raise SceneError(f"line {line_number}: background expects one #RRGGBB color")
            background = None if args[0].lower() in {"auto", "none"} else args[0]
        elif statement in {"layer", "family"}:
            if not args:
                raise SceneError(f"line {line_number}: layer expects a family name")
            family = normalize_family(args[0])
            assignments = _assignments(args[1:], line_number)
            alpha = assignments.pop("alpha", 1.0)
            blend = str(assignments.pop("blend", "normal")).lower()
            visible = assignments.pop("visible", True)
            if isinstance(alpha, bool):
                raise SceneError(f"line {line_number}: alpha must be numeric")
            if not isinstance(visible, bool):
                raise SceneError(f"line {line_number}: visible must be true or false")
            base = default_layer(family)
            params = dict(base.params)
            params.update(assignments)
            layers.append(
                Layer(
                    family=family,
                    params=params,
                    alpha=ensure_finite_number(alpha, f"line {line_number} alpha"),
                    blend=blend,
                    visible=visible,
                )
            )
        elif statement == "animate":
            if not args:
                raise SceneError(f"line {line_number}: animate expects a target")
            target = args[0]
            assignments = _assignments(args[1:], line_number)
            unknown = set(assignments) - {
                "from",
                "to",
                "seconds",
                "easing",
                "loop",
                "pingpong",
            }
            if unknown:
                raise SceneError(
                    f"line {line_number}: unsupported animation option(s): {', '.join(sorted(unknown))}"
                )
            if "from" not in assignments or "to" not in assignments:
                raise SceneError(f"line {line_number}: animate requires from= and to=")
            if not isinstance(assignments.get("loop", True), bool) or not isinstance(
                assignments.get("pingpong", False), bool
            ):
                raise SceneError(
                    f"line {line_number}: loop and pingpong must be true or false"
                )
            try:
                animations.append(
                    Animation(
                        target=target,
                        start=float(assignments["from"]),
                        end=float(assignments["to"]),
                        seconds=float(assignments.get("seconds", 8.0)),
                        easing=str(assignments.get("easing", "sine")).lower(),
                        loop=assignments.get("loop", True),
                        pingpong=assignments.get("pingpong", False),
                    )
                )
            except (TypeError, ValueError) as exc:
                raise SceneError(f"line {line_number}: invalid animation value") from exc
        else:
            raise SceneError(f"line {line_number}: unsupported statement '{parts[0]}'")

    scene = Scene(
        width=width,
        height=height,
        seed=seed,
        palette=palette,
        background=background,
        layers=tuple(layers),
        animations=tuple(animations),
        name=name,
        version=version,
    )
    return validate_scene(scene)


def _format_value(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, float):
        return repr(value)
    return str(value)


def format_live_code(scene: Scene) -> str:
    scene = validate_scene(scene)
    lines = [
        f"version {scene.version}",
        f"name {shlex.quote(scene.name)}",
        f"canvas {scene.width} {scene.height}",
        f"seed {scene.seed}",
        f"palette {scene.palette}",
    ]
    if scene.background is not None:
        lines.append(f"background {scene.background}")
    lines.append("")
    for layer in scene.layers:
        values = [f"{key}={_format_value(value)}" for key, value in sorted(layer.params.items())]
        if layer.alpha != 1.0:
            values.append(f"alpha={_format_value(layer.alpha)}")
        if layer.blend != "normal":
            values.append(f"blend={layer.blend}")
        if not layer.visible:
            values.append("visible=false")
        lines.append(" ".join(["layer", layer.family, *values]))
    for animation in scene.animations:
        lines.append(
            " ".join(
                [
                    "animate",
                    animation.target,
                    f"from={_format_value(animation.start)}",
                    f"to={_format_value(animation.end)}",
                    f"seconds={_format_value(animation.seconds)}",
                    f"easing={animation.easing}",
                    f"loop={_format_value(animation.loop)}",
                    f"pingpong={_format_value(animation.pingpong)}",
                ]
            )
        )
    return "\n".join(lines).rstrip() + "\n"


def mutate_scene(scene: Scene, mutation_index: int, strength: float = 0.25) -> Scene:
    """Create a reproducible variation; no wall-clock randomness is involved."""

    scene = validate_scene(scene)
    if mutation_index < 0:
        raise SceneError("mutation index must be non-negative")
    strength = max(0.02, min(1.0, float(strength)))
    digest = hashlib.sha256(
        f"{scene.recipe_hash()}:{mutation_index}".encode("ascii")
    ).digest()
    rng = random.Random(int.from_bytes(digest[:16], "big"))

    palettes = sorted(PALETTES)
    palette = scene.palette
    candidates = [item for item in palettes if item != palette]
    if candidates:
        palette = rng.choice(candidates)

    layers = list(scene.layers)
    target_index = rng.randrange(len(layers))
    target = layers[target_index]
    family_spec = FAMILY_SPECS[target.family]
    params = dict(target.params)
    mutable = [
        (name, spec)
        for name, spec in family_spec.params.items()
        if spec.kind in {"int", "float", "bool", "enum"}
    ]
    rng.shuffle(mutable)
    changes = 0
    for name, spec in mutable:
        current = params[name]
        if spec.kind == "bool":
            params[name] = not bool(current)
        elif spec.kind == "enum":
            alternatives = [value for value in spec.choices if value != current]
            if not alternatives:
                continue
            params[name] = rng.choice(alternatives)
        else:
            low = float(spec.minimum if spec.minimum is not None else float(current) - 1.0)
            high = float(spec.maximum if spec.maximum is not None else float(current) + 1.0)
            span = high - low
            delta = span * strength * rng.uniform(-0.45, 0.45)
            if abs(delta) < span * 0.01:
                delta = span * 0.05 * (1 if rng.random() > 0.5 else -1)
            value = max(low, min(high, float(current) + delta))
            params[name] = int(round(value)) if spec.kind == "int" else value
        if params[name] != current:
            changes += 1
        if changes >= 3:
            break
    layers[target_index] = replace(target, params=params)
    base_name = re.sub(r" · variation \d+$", "", scene.name)
    suffix = f" · variation {mutation_index + 1}"
    base_name = base_name[: max(1, 120 - len(suffix))]
    varied = replace(
        scene,
        seed=int.from_bytes(digest[16:24], "big") & ((1 << 63) - 1),
        palette=palette,
        layers=tuple(layers),
        name=base_name + suffix,
    )
    return validate_scene(varied)

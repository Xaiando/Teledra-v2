"""Validated capabilities registry for Fractus v2.

This file is the engine's single source of truth.  The DSL, GUI, renderer, and
headless validator all consume the same family and palette definitions.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
import re
from typing import Any, Mapping

from fractus_model import Layer, Scene, SceneError, ensure_finite_number, validate_model_bounds


@dataclass(frozen=True)
class ParamSpec:
    kind: str
    default: Any
    minimum: float | None = None
    maximum: float | None = None
    choices: tuple[str, ...] = ()

    def coerce(self, value: Any, field_name: str) -> Any:
        if self.kind == "bool":
            if isinstance(value, bool):
                return value
            lowered = str(value).strip().lower()
            if lowered in {"true", "yes", "on", "1"}:
                return True
            if lowered in {"false", "no", "off", "0"}:
                return False
            raise SceneError(f"{field_name} must be true or false")
        if self.kind == "enum":
            normalized = str(value).strip().lower().replace("-", "_")
            if normalized not in self.choices:
                raise SceneError(
                    f"{field_name} must be one of: {', '.join(self.choices)}"
                )
            return normalized
        if self.kind == "int":
            if not isinstance(value, int) or isinstance(value, bool):
                raise SceneError(f"{field_name} must be an integer")
            number = value
            if self.minimum is not None and number < self.minimum:
                raise SceneError(f"{field_name} must be at least {self.minimum:g}")
            if self.maximum is not None and number > self.maximum:
                raise SceneError(f"{field_name} must be at most {self.maximum:g}")
            return number
        number = ensure_finite_number(value, field_name)
        if self.minimum is not None and number < self.minimum:
            raise SceneError(f"{field_name} must be at least {self.minimum:g}")
        if self.maximum is not None and number > self.maximum:
            raise SceneError(f"{field_name} must be at most {self.maximum:g}")
        return float(number)


@dataclass(frozen=True)
class FamilySpec:
    family: str
    label: str
    category: str
    renderer: str
    params: Mapping[str, ParamSpec]
    stochastic: bool = False


I = lambda default, low, high: ParamSpec("int", default, low, high)
F = lambda default, low, high: ParamSpec("float", default, low, high)
B = lambda default: ParamSpec("bool", default)
E = lambda default, *choices: ParamSpec("enum", default, choices=tuple(choices))

VIEW = {
    "center_x": F(-0.75, -5.0, 5.0),
    "center_y": F(0.0, -5.0, 5.0),
    "scale": F(3.0, 1e-7, 10.0),
}
CURVE = {
    "density": I(48, 4, 160),
    "samples": I(2400, 200, 20_000),
    "phase": F(0.0, -50.0, 50.0),
    "line_width": I(1, 1, 8),
}


def _spec(
    family: str,
    label: str,
    category: str,
    renderer: str,
    params: Mapping[str, ParamSpec],
    stochastic: bool = False,
) -> FamilySpec:
    return FamilySpec(family, label, category, renderer, dict(params), stochastic)


FAMILY_SPECS: dict[str, FamilySpec] = {
    # Escape-time and convergence fractals.
    "mandelbrot": _spec("mandelbrot", "Mandelbrot", "Complex Fractals", "escape", {**VIEW, "iterations": I(180, 20, 800), "power": I(2, 2, 8), "bailout": F(4.0, 2.0, 100.0)}),
    "julia": _spec("julia", "Julia", "Complex Fractals", "escape", {**VIEW, "center_x": F(0.0, -5.0, 5.0), "iterations": I(180, 20, 800), "power": I(2, 2, 8), "bailout": F(4.0, 2.0, 100.0), "c_real": F(-0.7, -2.5, 2.5), "c_imag": F(0.27015, -2.5, 2.5)}),
    "multibrot": _spec("multibrot", "Multibrot", "Complex Fractals", "escape", {**VIEW, "iterations": I(180, 20, 800), "power": I(3, 2, 8), "bailout": F(8.0, 2.0, 100.0)}),
    "burning_ship": _spec("burning_ship", "Burning Ship", "Complex Fractals", "escape", {**VIEW, "center_x": F(-0.5, -5.0, 5.0), "center_y": F(-0.5, -5.0, 5.0), "iterations": I(200, 20, 800), "bailout": F(4.0, 2.0, 100.0)}),
    "tricorn": _spec("tricorn", "Tricorn", "Complex Fractals", "escape", {**VIEW, "iterations": I(180, 20, 800), "bailout": F(4.0, 2.0, 100.0)}),
    "newton": _spec("newton", "Newton Basins", "Complex Fractals", "newton", {"center_x": F(0.0, -5.0, 5.0), "center_y": F(0.0, -5.0, 5.0), "scale": F(3.6, 1e-7, 10.0), "iterations": I(80, 10, 400), "degree": I(3, 3, 8), "tolerance": F(1e-5, 1e-10, 0.1)}),
    # Radial and mandala geometry.
    "mandala": _spec("mandala", "Harmonic Mandala", "Mandalas", "polar", {"symmetry": I(12, 3, 48), "rings": I(8, 1, 24), "twist": F(0.35, -4.0, 4.0), "lace": F(0.55, 0.0, 2.0), "contrast": F(1.2, 0.2, 4.0)}),
    "lotus_mandala": _spec("lotus_mandala", "Lotus Mandala", "Mandalas", "polar", {"symmetry": I(16, 4, 48), "rings": I(7, 2, 24), "twist": F(0.18, -3.0, 3.0), "lace": F(0.8, 0.0, 2.0), "contrast": F(1.4, 0.2, 4.0)}),
    "star_mandala": _spec("star_mandala", "Nested Star Mandala", "Mandalas", "star_mandala", {"points": I(12, 5, 40), "rings": I(10, 2, 30), "step": I(5, 2, 19), "rotation": F(0.0, -20.0, 20.0), "line_width": I(2, 1, 8)}),
    "flower_of_life": _spec("flower_of_life", "Flower of Life", "Mandalas", "flower", {"rings": I(5, 1, 12), "circle_scale": F(1.0, 0.3, 2.0), "rotation": F(0.0, -20.0, 20.0), "line_width": I(2, 1, 8)}),
    "radial_weave": _spec("radial_weave", "Radial Weave", "Mandalas", "polar", {"symmetry": I(18, 3, 64), "rings": I(11, 2, 32), "twist": F(1.1, -5.0, 5.0), "lace": F(1.35, 0.0, 3.0), "contrast": F(1.6, 0.2, 5.0)}),
    "kaleidoscope": _spec("kaleidoscope", "Kaleidoscope", "Mandalas", "polar", {"symmetry": I(14, 3, 64), "rings": I(9, 2, 32), "twist": F(-0.55, -5.0, 5.0), "lace": F(1.1, 0.0, 3.0), "contrast": F(2.0, 0.2, 5.0)}),
    "phyllotaxis": _spec("phyllotaxis", "Phyllotaxis Bloom", "Mandalas", "phyllotaxis", {"points": I(1800, 100, 30_000), "angle": F(137.507764, 1.0, 359.0), "dot_size": F(1.8, 0.2, 12.0), "spiral": F(0.48, 0.1, 1.2)}, True),
    # Harmonic curves and optical line work.
    "woven_web": _spec("woven_web", "Woven Web", "Curves & Optical", "curves", {**CURVE, "a": F(5.0, 1.0, 32.0), "b": F(7.0, 1.0, 32.0), "warp": F(0.18, 0.0, 1.0)}),
    "guilloche": _spec("guilloche", "Guilloche", "Curves & Optical", "curves", {**CURVE, "ratio": F(6.2, 0.2, 32.0), "offset": F(0.42, 0.0, 1.2), "warp": F(0.12, 0.0, 1.0)}),
    "lissajous": _spec("lissajous", "Lissajous Lace", "Curves & Optical", "curves", {**CURVE, "a": F(5.0, 1.0, 40.0), "b": F(8.0, 1.0, 40.0), "warp": F(0.08, 0.0, 1.0)}),
    "moire": _spec("moire", "Moire Field", "Curves & Optical", "moire", {"density": I(62, 8, 180), "frequency": F(12.0, 1.0, 80.0), "warp": F(0.08, 0.0, 0.5), "rotation": F(0.16, -3.2, 3.2), "line_width": I(1, 1, 6)}),
    "orbital_lace": _spec("orbital_lace", "Orbital Lace", "Curves & Optical", "orbital", {"density": I(54, 8, 140), "holes": I(7, 1, 18), "swirl": F(0.021, -0.2, 0.2), "repel": F(0.028, -0.2, 0.2), "line_width": I(1, 1, 6)}, True),
    "spirograph": _spec("spirograph", "Spirograph", "Curves & Optical", "curves", {**CURVE, "ratio": F(5.4, 1.1, 30.0), "offset": F(0.62, 0.0, 1.5), "warp": F(0.0, 0.0, 1.0)}),
    "harmonograph": _spec("harmonograph", "Harmonograph", "Curves & Optical", "harmonograph", {"samples": I(12_000, 500, 60_000), "frequency_x": F(3.01, 0.1, 20.0), "frequency_y": F(2.0, 0.1, 20.0), "phase": F(1.2, -20.0, 20.0), "damping": F(0.018, 0.0001, 0.2), "line_width": I(1, 1, 6)}),
    "rose_curve": _spec("rose_curve", "Rose Curve", "Curves & Optical", "curves", {**CURVE, "a": F(7.0, 1.0, 40.0), "b": F(1.0, 0.2, 10.0), "warp": F(0.25, 0.0, 1.0)}),
    "string_art": _spec("string_art", "String Art", "Curves & Optical", "string_art", {"pins": I(120, 12, 600), "step": I(37, 2, 299), "cycles": I(4, 1, 20), "radius": F(0.92, 0.1, 1.0), "line_width": I(1, 1, 5)}),
    # Recursive geometry and iterated systems.
    "sierpinski": _spec("sierpinski", "Sierpinski Triangle", "Recursive Geometry", "sierpinski", {"depth": I(7, 1, 10), "filled": B(False), "line_width": I(1, 1, 6)}),
    "koch_snowflake": _spec("koch_snowflake", "Koch Snowflake", "Recursive Geometry", "koch", {"depth": I(5, 0, 7), "inward": B(False), "line_width": I(2, 1, 8)}),
    "fractal_tree": _spec("fractal_tree", "Fractal Tree", "Recursive Geometry", "tree", {"depth": I(11, 2, 14), "angle": F(25.0, 1.0, 89.0), "shrink": F(0.72, 0.45, 0.86), "jitter": F(4.0, 0.0, 20.0), "line_width": I(2, 1, 10)}, True),
    "dragon_curve": _spec("dragon_curve", "Dragon Curve", "Recursive Geometry", "dragon", {"depth": I(14, 4, 18), "rotation": F(0.0, -20.0, 20.0), "line_width": I(2, 1, 8)}),
    "barnsley_fern": _spec("barnsley_fern", "Barnsley Fern", "Recursive Geometry", "fern", {"points": I(90_000, 2_000, 400_000), "spread": F(0.9, 0.3, 1.4), "dot_size": I(1, 1, 4)}, True),
    "l_system": _spec("l_system", "L-System Garden", "Recursive Geometry", "l_system", {"preset": E("plant", "plant", "koch", "hilbert", "gosper"), "depth": I(5, 1, 7), "angle": F(25.0, 5.0, 120.0), "line_width": I(1, 1, 8)}),
    # Tessellation and op-art.
    "truchet": _spec("truchet", "Truchet Tiles", "Tessellation & Op Art", "truchet", {"tiles": I(18, 3, 80), "line_width": I(3, 1, 16), "double": B(True), "rotation_bias": F(0.5, 0.0, 1.0)}, True),
    "hex_weave": _spec("hex_weave", "Hex Weave", "Tessellation & Op Art", "hex", {"columns": I(16, 3, 60), "rings": I(2, 1, 6), "twist": F(0.2, -3.2, 3.2), "line_width": I(2, 1, 10)}),
    "op_art": _spec("op_art", "Optical Wave Grid", "Tessellation & Op Art", "op_art", {"density": I(48, 8, 160), "frequency": F(7.0, 0.5, 40.0), "warp": F(0.22, 0.0, 1.0), "rotation": F(0.0, -3.2, 3.2), "line_width": I(1, 1, 8)}),
    # Dynamic and field systems.
    "cellular_automata": _spec("cellular_automata", "Cellular Automata", "Dynamic Fields", "cellular", {"rule": I(90, 0, 255), "rows": I(512, 32, 2048), "random_start": B(False), "mirror": B(True)}, True),
    "reaction_diffusion": _spec("reaction_diffusion", "Reaction Diffusion", "Dynamic Fields", "reaction", {"iterations": I(160, 20, 600), "feed": F(0.055, 0.0, 0.1), "kill": F(0.062, 0.0, 0.1), "diff_a": F(1.0, 0.1, 2.0), "diff_b": F(0.5, 0.1, 2.0), "spots": I(9, 1, 80)}, True),
    "flow_field": _spec("flow_field", "Flow Field", "Dynamic Fields", "flow", {"streams": I(900, 20, 8_000), "steps": I(90, 5, 400), "frequency": F(2.8, 0.1, 20.0), "curl": F(1.1, -6.0, 6.0), "step_size": F(0.012, 0.001, 0.08), "line_width": I(1, 1, 5)}, True),
    "strange_attractor": _spec("strange_attractor", "Strange Attractor", "Dynamic Fields", "attractor", {"points": I(160_000, 5_000, 600_000), "a": F(-1.4, -3.0, 3.0), "b": F(1.6, -3.0, 3.0), "c": F(1.0, -3.0, 3.0), "d": F(0.7, -3.0, 3.0), "dot_size": I(1, 1, 4)}),

    # Upgrade: animated particles with pseudo-3D (perspective, depth, rotation). Good for "green particle that almost looked 3D".
    "particles": _spec("particles", "Particles (3D-ish)", "Procedural", "particles", {
        **VIEW,
        "count": I(120, 20, 800),
        "speed": F(1.2, 0.2, 4.0),
        "size": F(1.5, 0.5, 6.0),
        "depth": F(2.0, 0.5, 6.0),
        "rotation": F(0.5, -4.0, 4.0),
        "phase": F(0.0, 0.0, 10.0),  # for animation: moves particles over time
        "hue_shift": F(0.0, -1.0, 1.0),
    }, True),
}


PALETTES: dict[str, tuple[str, ...]] = {
    "purple_haze": ("#08020e", "#1a0933", "#0064ff", "#b500ff", "#ff007f", "#fff0ff"),
    "electric_cyan": ("#020712", "#003250", "#009688", "#00e5ff", "#c8ffff"),
    "neon_sunset": ("#10030b", "#5e093f", "#ff4000", "#ffbf00", "#ffff80"),
    "emerald": ("#031007", "#053214", "#009650", "#00e676", "#dcffdc"),
    "twilight": ("#090719", "#29326f", "#7a4ea3", "#f08a9d", "#ffd9a0"),
    "rainbow": ("#12021d", "#5848ff", "#00d8ff", "#39ff88", "#ffe34e", "#ff633f", "#ff4fd8"),
    "pastel": ("#161323", "#a7c7e7", "#cdb4db", "#ffc8dd", "#ffafcc", "#fdf0d5"),
    "amethyst": ("#09040f", "#321450", "#663399", "#ad73d2", "#f0d9ff"),
    "monochrome": ("#030303", "#242424", "#747474", "#d0d0d0", "#ffffff"),
    "solar_gold": ("#100700", "#642200", "#c75b00", "#ffb000", "#fff0a8"),
    "ice_fire": ("#08102f", "#1769aa", "#d8ffff", "#ffdfb0", "#e52f2f", "#39020a"),
}

PALETTE_ALIASES = {
    "cyan": "electric_cyan",
    "blue_ocean": "electric_cyan",
    "sunset": "neon_sunset",
    "solar_flare": "solar_gold",
    "chrome": "monochrome",
}

FAMILY_ALIASES = {
    "burning-ship": "burning_ship",
    "sierpinski_triangle": "sierpinski",
    "cellular_automaton": "cellular_automata",
    "cellular": "cellular_automata",
    "reaction-diffusion": "reaction_diffusion",
    "koch": "koch_snowflake",
    "fractal_flower": "lotus_mandala",
    "lorenz": "strange_attractor",
    "lsystem": "l_system",
    "lindenmayer": "l_system",
}

_HEX = re.compile(r"^#[0-9a-fA-F]{6}$")
MAX_ESTIMATED_WORK = 400_000_000
MAX_ESTIMATED_MEMORY = 512 * 1024 * 1024


def normalize_family(value: str) -> str:
    normalized = str(value).strip().lower().replace(" ", "_")
    normalized = FAMILY_ALIASES.get(normalized, normalized.replace("-", "_"))
    if normalized not in FAMILY_SPECS:
        raise SceneError(
            f"unsupported Fractus family '{value}'. Available families: "
            + ", ".join(sorted(FAMILY_SPECS))
        )
    return normalized


def normalize_palette(value: str) -> str:
    value = str(value).strip()
    normalized = value.lower().replace(" ", "_").replace("-", "_")
    normalized = PALETTE_ALIASES.get(normalized, normalized)
    if normalized in PALETTES:
        return normalized
    stops = [stop.strip() for stop in value.split(",") if stop.strip()]
    if 2 <= len(stops) <= 8 and all(_HEX.match(stop) for stop in stops):
        return ",".join(stop.lower() for stop in stops)
    raise SceneError(
        f"unsupported Fractus palette '{value}'. Use a named palette or 2-8 comma-separated #RRGGBB colors"
    )


def palette_stops(value: str) -> tuple[str, ...]:
    normalized = normalize_palette(value)
    if normalized in PALETTES:
        return PALETTES[normalized]
    return tuple(normalized.split(","))


def default_layer(family: str) -> Layer:
    family = normalize_family(family)
    spec = FAMILY_SPECS[family]
    return Layer(family=family, params={key: item.default for key, item in spec.params.items()})


def validate_layer(layer: Layer, index: int = 0) -> Layer:
    family = normalize_family(layer.family)
    spec = FAMILY_SPECS[family]
    unknown = sorted(set(layer.params) - set(spec.params))
    if unknown:
        raise SceneError(
            f"layer {index} family '{family}' does not support parameter(s): {', '.join(unknown)}"
        )
    params: dict[str, Any] = {}
    for name, param_spec in spec.params.items():
        value = layer.params.get(name, param_spec.default)
        params[name] = param_spec.coerce(value, f"layer {index} {name}")
    return replace(layer, family=family, params=params)


def validate_scene(scene: Scene) -> Scene:
    validate_model_bounds(scene)
    palette = normalize_palette(scene.palette)
    layers = tuple(validate_layer(layer, index) for index, layer in enumerate(scene.layers))
    normalized = replace(scene, palette=palette, layers=layers)
    # Animation values are validated against their target parameter ranges.
    for animation in normalized.animations:
        parts = animation.target.split(".", 1)
        if len(parts) != 2 or not parts[0].isdigit():
            raise SceneError(
                f"animation target '{animation.target}' must be <layer-index>.<parameter>"
            )
        layer_index = int(parts[0])
        if not 0 <= layer_index < len(layers):
            raise SceneError(f"animation target layer {layer_index} does not exist")
        parameter = parts[1]
        family_spec = FAMILY_SPECS[layers[layer_index].family]
        if parameter not in family_spec.params:
            raise SceneError(
                f"animation target '{animation.target}' is not a parameter of {layers[layer_index].family}"
            )
        spec = family_spec.params[parameter]
        if spec.kind != "float":
            raise SceneError(
                f"animation target '{animation.target}' must be a continuous float parameter"
            )
        spec.coerce(animation.start, f"animation {animation.target} start")
        spec.coerce(animation.end, f"animation {animation.target} end")
    work = sum(_estimated_layer_work(normalized, layer) for layer in layers if layer.visible)
    if work > MAX_ESTIMATED_WORK:
        raise SceneError(
            f"scene exceeds the {MAX_ESTIMATED_WORK:,}-unit render-work budget "
            f"(estimated {work:,}); reduce canvas, iterations, density, samples, or layers"
        )
    peak_memory = max(
        (_estimated_layer_memory(normalized, layer) for layer in layers if layer.visible),
        default=0,
    )
    if peak_memory > MAX_ESTIMATED_MEMORY:
        raise SceneError(
            f"scene exceeds the {MAX_ESTIMATED_MEMORY // (1024 * 1024)} MB render-memory budget "
            f"(estimated {peak_memory / (1024 * 1024):.1f} MB); reduce canvas or Newton degree"
        )
    return normalized


def _estimated_layer_work(scene: Scene, layer: Layer) -> int:
    renderer = FAMILY_SPECS[layer.family].renderer
    p = layer.params
    pixels = scene.width * scene.height
    longest = max(scene.width, scene.height)
    if renderer == "escape":
        return pixels * int(p["iterations"])
    if renderer == "newton":
        return pixels * int(p["iterations"]) * (int(p["degree"]) + 2)
    if renderer in {"polar", "cellular"}:
        return pixels * 4
    if renderer in {"curves"}:
        return int(p["density"]) * int(p["samples"]) * 16
    if renderer in {"moire", "op_art"}:
        return int(p["density"]) * longest * 4
    if renderer == "orbital":
        return int(p["density"]) * longest * int(p["holes"]) * 10
    if renderer == "harmonograph":
        return int(p["samples"])
    if renderer == "string_art":
        return int(p["pins"]) * int(p["cycles"])
    if renderer in {"fern", "phyllotaxis", "attractor"}:
        return int(p["points"])
    if renderer == "flow":
        return int(p["streams"]) * int(p["steps"])
    if renderer == "reaction":
        simulation_size = max(64, min(192, min(scene.width, scene.height)))
        return simulation_size * simulation_size * int(p["iterations"]) * 12
    if renderer in {"tree", "sierpinski"}:
        return 3 ** int(p["depth"])
    if renderer in {"dragon"}:
        return 2 ** int(p["depth"])
    if renderer in {"koch"}:
        return 4 ** int(p["depth"])
    if renderer == "l_system":
        return 8 ** int(p["depth"])
    return pixels * 2


def _estimated_layer_memory(scene: Scene, layer: Layer) -> int:
    pixels = scene.width * scene.height
    renderer = FAMILY_SPECS[layer.family].renderer
    # Includes the output canvas and the largest renderer-specific temporary.
    if renderer == "newton":
        return pixels * (72 + 8 * int(layer.params["degree"]))
    if renderer == "escape":
        # Escape rendering is row-chunked, so its peak does not scale like a
        # full collection of complex work arrays.
        return pixels * 12 + scene.width * min(scene.height, 48) * 80
    if renderer in {"reaction", "cellular", "attractor", "fern"}:
        return pixels * 32
    return pixels * 16


def legacy_scene(
    family: str = "mandelbrot",
    iterations: int = 150,
    palette: str = "purple_haze",
    c_real: float = -0.7,
    c_imag: float = 0.27015,
    seed: int = 1,
    width: int = 600,
    height: int = 600,
) -> Scene:
    """Map the original single-layer CLI contract into a typed v2 scene."""

    family = normalize_family(family)
    palette = normalize_palette(palette)
    detail = int(ensure_finite_number(iterations, "iterations"))
    if not 20 <= detail <= 800:
        raise SceneError("iterations must be between 20 and 800")
    layer = default_layer(family)
    params = dict(layer.params)
    if "iterations" in params:
        spec = FAMILY_SPECS[family].params["iterations"]
        params["iterations"] = max(int(spec.minimum or 0), min(int(spec.maximum or detail), detail))
    if "density" in params:
        spec = FAMILY_SPECS[family].params["density"]
        params["density"] = max(
            int(spec.minimum or 0),
            min(int(spec.maximum or 160), 16 + detail // 5),
        )
    if "samples" in params:
        spec = FAMILY_SPECS[family].params["samples"]
        params["samples"] = max(
            int(spec.minimum or 0),
            min(int(spec.maximum or 20_000), detail * 18),
        )
    if "rings" in params:
        spec = FAMILY_SPECS[family].params["rings"]
        params["rings"] = max(
            int(spec.minimum or 0),
            min(int(spec.maximum or 24), 3 + detail // 40),
        )
    if "symmetry" in params:
        spec = FAMILY_SPECS[family].params["symmetry"]
        params["symmetry"] = max(
            int(spec.minimum or 0),
            min(int(spec.maximum or 48), 8 + detail % 17),
        )
    if "depth" in params:
        spec = FAMILY_SPECS[family].params["depth"]
        params["depth"] = max(
            int(spec.minimum or 0),
            min(int(spec.maximum or 14), 3 + detail // 70),
        )
    if "points" in params:
        spec = FAMILY_SPECS[family].params["points"]
        params["points"] = max(
            int(spec.minimum or 0),
            min(int(spec.maximum or 400_000), detail * 450),
        )
    if "c_real" in params:
        params["c_real"] = ensure_finite_number(c_real, "c_real")
    if "c_imag" in params:
        params["c_imag"] = ensure_finite_number(c_imag, "c_imag")
    # Preserve the old C controls as useful phase/twist variation for line art.
    if family not in {"julia"}:
        if "phase" in params:
            phase_spec = FAMILY_SPECS[family].params["phase"]
            phase = ensure_finite_number(c_real, "c_real")
            if phase_spec.minimum is not None:
                phase = max(float(phase_spec.minimum), phase)
            if phase_spec.maximum is not None:
                phase = min(float(phase_spec.maximum), phase)
            params["phase"] = phase
        if "twist" in params:
            params["twist"] = max(-5.0, min(5.0, ensure_finite_number(c_real, "c_real")))
        if "warp" in params:
            params["warp"] = max(0.0, min(1.0, abs(ensure_finite_number(c_imag, "c_imag"))))
    return validate_scene(
        Scene(
            width=int(width),
            height=int(height),
            seed=int(seed),
            palette=palette,
            layers=(replace(layer, params=params),),
            name=f"Legacy {FAMILY_SPECS[family].label}",
        )
    )


def capability_manifest() -> dict[str, Any]:
    return {
        "schema_version": 2,
        "limits": {
            "max_layers": 8,
            "canvas": [64, 2048],
            "legacy_iterations": [20, 800],
            "max_estimated_work": MAX_ESTIMATED_WORK,
            "max_estimated_memory": MAX_ESTIMATED_MEMORY,
        },
        "palettes": sorted(PALETTES),
        "palette_aliases": dict(sorted(PALETTE_ALIASES.items())),
        "families": {
            family: {
                "label": spec.label,
                "category": spec.category,
                "stochastic": spec.stochastic,
                "parameters": {
                    name: {
                        "kind": item.kind,
                        "default": item.default,
                        "minimum": item.minimum,
                        "maximum": item.maximum,
                        "choices": list(item.choices),
                    }
                    for name, item in spec.params.items()
                },
            }
            for family, spec in sorted(FAMILY_SPECS.items())
        },
    }


def families_by_category() -> dict[str, list[str]]:
    result: dict[str, list[str]] = {}
    for family, spec in FAMILY_SPECS.items():
        result.setdefault(spec.category, []).append(family)
    for families in result.values():
        families.sort()
    return dict(sorted(result.items()))

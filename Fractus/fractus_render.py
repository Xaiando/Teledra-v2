"""Pure deterministic renderers for the Fractus v2 family registry."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import random
import time
from typing import Any, Callable
import uuid

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter

from fractus_model import Layer, Scene, SceneError, scene_for_frame, Animation
from fractus_registry import FAMILY_SPECS, palette_stops, validate_scene


ProgressCallback = Callable[[float, str], None]
CancelCallback = Callable[[], bool]


class RenderCancelled(RuntimeError):
    pass


@dataclass
class RenderResult:
    image: Image.Image
    recipe_hash: str
    render_hash: str
    metrics: dict[str, Any]
    duration_ms: int
    frame_index: int

    def manifest(self, scene: Scene) -> dict[str, Any]:
        return {
            "schema_version": 2,
            "state": "completed",
            "recipe_hash": self.recipe_hash,
            "render_hash": self.render_hash,
            "duration_ms": self.duration_ms,
            "frame_index": self.frame_index,
            "width": self.image.width,
            "height": self.image.height,
            "metrics": self.metrics,
            "scene": scene.to_dict(),
        }


def _check_cancel(cancel: CancelCallback | None) -> None:
    if cancel is not None and cancel():
        raise RenderCancelled("render cancelled")


def _notify(progress: ProgressCallback | None, value: float, stage: str) -> None:
    if progress is not None:
        progress(max(0.0, min(1.0, value)), stage)


def _rgb(value: str) -> tuple[int, int, int]:
    value = value.lstrip("#")
    return tuple(int(value[index : index + 2], 16) for index in (0, 2, 4))


def _palette_lut(name: str, size: int = 1024) -> np.ndarray:
    colors = np.asarray([_rgb(stop) for stop in palette_stops(name)], dtype=np.float64)
    source = np.linspace(0.0, 1.0, len(colors))
    target = np.linspace(0.0, 1.0, size)
    channels = [np.interp(target, source, colors[:, channel]) for channel in range(3)]
    return np.column_stack(channels).clip(0, 255).astype(np.uint8)


def _line_colors(name: str) -> list[tuple[int, int, int]]:
    stops = list(palette_stops(name))
    return [_rgb(color) for color in stops[1:]] or [_rgb(stops[0])]


def _mixed_seed(scene_seed: int, index: int, family: str) -> int:
    digest = hashlib.sha256(f"{scene_seed}:{index}:{family}".encode("ascii")).digest()
    return int.from_bytes(digest[:8], "big")


def _transparent(width: int, height: int) -> Image.Image:
    return Image.new("RGBA", (width, height), (0, 0, 0, 0))


def _points_to_pixels(
    xs: np.ndarray,
    ys: np.ndarray,
    width: int,
    height: int,
    margin: float = 0.04,
) -> list[tuple[int, int]]:
    usable_x = width * (1.0 - margin * 2.0)
    usable_y = height * (1.0 - margin * 2.0)
    px = width * margin + (xs + 1.0) * 0.5 * usable_x
    py = height * margin + (1.0 - (ys + 1.0) * 0.5) * usable_y
    mask = np.isfinite(px) & np.isfinite(py)
    return list(zip(px[mask].astype(int).tolist(), py[mask].astype(int).tolist()))


def _scalar_image(
    values: np.ndarray,
    palette: str,
    mask: np.ndarray | None = None,
) -> Image.Image:
    values = np.nan_to_num(values, nan=0.0, posinf=1.0, neginf=0.0)
    values = np.clip(values, 0.0, 1.0)
    lut = _palette_lut(palette)
    indices = np.minimum((values * (len(lut) - 1)).astype(np.int32), len(lut) - 1)
    rgb = lut[indices]
    alpha = np.full(values.shape, 255, dtype=np.uint8)
    if mask is not None:
        alpha = np.where(mask, 255, 0).astype(np.uint8)
    return Image.fromarray(np.dstack([rgb, alpha]), "RGBA")


def _render_escape(
    scene: Scene,
    layer: Layer,
    progress: ProgressCallback | None,
    cancel: CancelCallback | None,
) -> Image.Image:
    p = layer.params
    width, height = scene.width, scene.height
    center_x, center_y, scale = p["center_x"], p["center_y"], p["scale"]
    x_scale = scale * width / height
    xs = np.linspace(center_x - x_scale / 2.0, center_x + x_scale / 2.0, width)
    ys = np.linspace(center_y + scale / 2.0, center_y - scale / 2.0, height)
    output = np.zeros((height, width, 4), dtype=np.uint8)
    lut = _palette_lut(scene.palette)
    iterations = int(p["iterations"])
    bailout = float(p["bailout"])
    power = int(p.get("power", 2))
    chunk = max(8, min(48, height // 16 or 8))

    for y0 in range(0, height, chunk):
        _check_cancel(cancel)
        y1 = min(height, y0 + chunk)
        coordinates = xs[None, :] + 1j * ys[y0:y1, None]
        if layer.family == "julia":
            z = coordinates.copy()
            constant: complex | np.ndarray = complex(p["c_real"], p["c_imag"])
        else:
            z = np.zeros_like(coordinates)
            constant = coordinates
        active = np.ones(z.shape, dtype=bool)
        smooth = np.zeros(z.shape, dtype=np.float64)
        escaped_any = np.zeros(z.shape, dtype=bool)
        with np.errstate(over="ignore", invalid="ignore", divide="ignore"):
            for iteration in range(iterations):
                if iteration % 20 == 0:
                    _check_cancel(cancel)
                if not np.any(active):
                    break
                current = z[active]
                if layer.family == "burning_ship":
                    current = np.abs(current.real) + 1j * np.abs(current.imag)
                elif layer.family == "tricorn":
                    current = np.conj(current)
                value = current**power + (constant if np.isscalar(constant) else constant[active])
                z[active] = value
                magnitude = np.abs(z)
                escaped = active & (magnitude > bailout)
                if np.any(escaped):
                    safe_mag = np.maximum(magnitude[escaped], 1.0000001)
                    smooth[escaped] = iteration + 1.0 - np.log(np.log(safe_mag)) / math.log(power)
                    escaped_any[escaped] = True
                    active[escaped] = False
        normalized = np.clip(smooth / max(iterations, 1), 0.0, 1.0) ** 0.32
        color_index = np.minimum((normalized * (len(lut) - 1)).astype(np.int32), len(lut) - 1)
        output[y0:y1, :, :3] = lut[color_index]
        output[y0:y1, :, 3] = np.where(escaped_any, 255, 0).astype(np.uint8)
        _notify(progress, y1 / height, f"{layer.family}: rows {y1}/{height}")
    return Image.fromarray(output, "RGBA")


def _render_newton(
    scene: Scene,
    layer: Layer,
    progress: ProgressCallback | None,
    cancel: CancelCallback | None,
) -> Image.Image:
    p = layer.params
    width, height = scene.width, scene.height
    scale = p["scale"]
    x_scale = scale * width / height
    xs = np.linspace(p["center_x"] - x_scale / 2, p["center_x"] + x_scale / 2, width)
    ys = np.linspace(p["center_y"] + scale / 2, p["center_y"] - scale / 2, height)
    z = xs[None, :] + 1j * ys[:, None]
    degree = int(p["degree"])
    iterations = int(p["iterations"])
    tolerance = p["tolerance"]
    roots = np.exp(2j * np.pi * np.arange(degree) / degree)
    root_id = np.full(z.shape, -1, dtype=np.int16)
    converged_at = np.full(z.shape, iterations, dtype=np.int16)
    active = np.ones(z.shape, dtype=bool)
    with np.errstate(over="ignore", invalid="ignore", divide="ignore"):
        for iteration in range(iterations):
            if iteration % 4 == 0:
                _check_cancel(cancel)
                _notify(progress, iteration / iterations, f"newton: iteration {iteration}")
            denominator = degree * z ** (degree - 1)
            safe_active = np.abs(denominator[active]) > 1e-14
            if np.any(safe_active):
                update = np.zeros(z.shape, dtype=bool)
                update[active] = safe_active
                z[update] -= (z[update] ** degree - 1.0) / denominator[update]
            
            active_z = z[active]
            if len(active_z) == 0:
                break
            
            distances = np.abs(active_z[..., None] - roots)
            nearest = np.argmin(distances, axis=1)
            minimum = np.take_along_axis(distances, nearest[..., None], axis=1)[..., 0]
            
            newly_active = minimum < tolerance
            if np.any(newly_active):
                newly = np.zeros(z.shape, dtype=bool)
                newly[active] = newly_active
                root_id[newly] = nearest[newly_active]
                converged_at[newly] = iteration
                active[newly] = False
            
            active &= np.isfinite(z.real) & np.isfinite(z.imag)
            if not np.any(active):
                break
    lut = _palette_lut(scene.palette)
    anchors = lut[np.linspace(160, len(lut) - 1, degree).astype(int)]
    brightness = 0.25 + 0.75 * (1.0 - converged_at / max(iterations, 1))
    rgb = np.zeros((height, width, 3), dtype=np.uint8)
    mask = root_id >= 0
    for root_index in range(degree):
        selected = root_id == root_index
        rgb[selected] = np.clip(anchors[root_index] * brightness[selected, None], 0, 255)
    alpha = np.where(mask, 255, 0).astype(np.uint8)
    _notify(progress, 1.0, "newton: basins resolved")
    return Image.fromarray(np.dstack([rgb, alpha]), "RGBA")


def _polar_grid(scene: Scene) -> tuple[np.ndarray, np.ndarray]:
    x = np.linspace(-1.0, 1.0, scene.width)
    y = np.linspace(1.0, -1.0, scene.height)
    xx, yy = np.meshgrid(x, y)
    return np.hypot(xx, yy), np.arctan2(yy, xx)


def _render_polar(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    _check_cancel(cancel)
    p = layer.params
    radius, theta = _polar_grid(scene)
    symmetry = int(p["symmetry"])
    rings = int(p["rings"])
    twist = float(p["twist"])
    lace = float(p["lace"])
    if layer.family == "lotus_mandala":
        petals = np.cos(symmetry * theta + twist * radius * math.tau)
        petal_ridges = np.exp(-10.0 * np.abs(radius - (0.2 + 0.63 * np.abs(petals))))
        ring_lace = 0.5 + 0.5 * np.cos(rings * math.tau * radius - lace * symmetry * theta)
        values = np.clip(petal_ridges * 0.72 + ring_lace * 0.28, 0.0, 1.0)
    elif layer.family == "kaleidoscope":
        folded = np.abs(((theta / math.tau * symmetry + 0.5) % 1.0) - 0.5) * 2.0
        values = 0.5 + 0.5 * np.sin(
            rings * math.tau * radius + twist * math.tau * folded + lace * np.cos(symmetry * theta)
        )
        values *= 0.45 + 0.55 * (1.0 - folded)
    else:
        harmonic = np.sin(symmetry * theta + twist * radius * math.tau)
        ring_wave = np.cos(rings * math.tau * radius + lace * np.sin(symmetry * theta * 0.5))
        cross = np.sin((symmetry // 2 + 2) * theta - (rings + 3) * radius * math.pi)
        if layer.family == "radial_weave":
            pattern = harmonic * ring_wave + 0.55 * cross
        else:
            pattern = 0.52 * harmonic + 0.32 * ring_wave + 0.16 * cross
        values = 0.5 + 0.5 * np.tanh(pattern * float(p["contrast"]))
    if layer.family in {"lotus_mandala", "kaleidoscope"}:
        values = 0.5 + 0.5 * np.tanh(
            (np.clip(values, 0.0, 1.0) - 0.5) * float(p["contrast"]) * 2.0
        )
    values *= np.clip((1.05 - radius) * 5.0, 0.0, 1.0)
    _check_cancel(cancel)
    return _scalar_image(values, scene.palette, radius <= 1.0)


def _render_star_mandala(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    points = int(p["points"])
    rings = int(p["rings"])
    step = int(p["step"]) % points or 1
    cx, cy = scene.width / 2.0, scene.height / 2.0
    maximum = min(scene.width, scene.height) * 0.47
    for ring in range(rings, 0, -1):
        _check_cancel(cancel)
        radius = maximum * ring / rings
        rotation = p["rotation"] + ring * math.pi / max(rings, 1)
        vertices = [
            (
                cx + radius * math.cos(rotation + math.tau * index / points),
                cy - radius * math.sin(rotation + math.tau * index / points),
            )
            for index in range(points)
        ]
        visited: set[int] = set()
        for start in range(points):
            if start in visited:
                continue
            order = []
            current = start
            while current not in visited:
                visited.add(current)
                order.append(vertices[current])
                current = (current + step) % points
            if len(order) >= 2:
                order.append(order[0])
                draw.line(
                    order,
                    fill=colors[(ring + start) % len(colors)]
                    + (110 + int(120 * ring / rings),),
                    width=int(p["line_width"]),
                    joint="curve",
                )
    return image


def _render_flower(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    rings = int(p["rings"])
    cx, cy = scene.width / 2.0, scene.height / 2.0
    spacing = min(scene.width, scene.height) * 0.43 / (rings + 1)
    radius = spacing * p["circle_scale"]
    centers = [(0.0, 0.0, 0)]
    for ring in range(1, rings + 1):
        _check_cancel(cancel)
        for side in range(6):
            for step in range(ring):
                angle = p["rotation"] + side * math.pi / 3.0
                next_angle = angle + math.pi / 3.0
                x = ring * math.cos(angle) + step * (math.cos(next_angle) - math.cos(angle))
                y = ring * math.sin(angle) + step * (math.sin(next_angle) - math.sin(angle))
                centers.append((x, y, ring))
    for x, y, ring in centers:
        px, py = cx + x * spacing, cy - y * spacing
        draw.ellipse(
            (px - radius, py - radius, px + radius, py + radius),
            outline=colors[ring % len(colors)] + (190,),
            width=int(p["line_width"]),
        )
    return image


def _render_phyllotaxis(
    scene: Scene, layer: Layer, seed: int, cancel: CancelCallback | None
) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    count = int(p["points"])
    angle = math.radians(p["angle"])
    phase = (seed % 10_000) / 10_000.0 * math.tau
    maximum = min(scene.width, scene.height) * 0.47
    dot = float(p["dot_size"])
    for index in range(count):
        if index % 256 == 0:
            _check_cancel(cancel)
        radius = maximum * (index / max(count - 1, 1)) ** p["spiral"]
        theta = index * angle + phase
        x = scene.width / 2 + radius * math.cos(theta)
        y = scene.height / 2 - radius * math.sin(theta)
        color = colors[(index * len(colors) // max(count, 1)) % len(colors)] + (210,)
        draw.ellipse((x - dot, y - dot, x + dot, y + dot), fill=color)
    return image


def _render_curves(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    samples = int(p["samples"])
    density = int(p["density"])
    t = np.linspace(0.0, math.tau, samples)
    for index in range(density):
        _check_cancel(cancel)
        phase = p["phase"] + index * math.tau / max(density, 1)
        if layer.family == "woven_web":
            envelope = 0.78 + p["warp"] * np.sin((5 + index % 5) * t + phase)
            xs = envelope * np.sin((p["a"] + index % 4) * t + phase)
            ys = envelope * np.sin((p["b"] + index % 6) * t - phase * 0.73)
        elif layer.family in {"guilloche", "spirograph"}:
            ratio = p["ratio"] + index * 0.015
            radius = 0.62 + 0.16 * math.sin(index * 0.37)
            offset = p["offset"]
            xs = radius * np.cos(t + phase) + offset * np.cos(ratio * t - phase)
            ys = radius * np.sin(t + phase) - offset * np.sin(ratio * t - phase)
            if layer.family == "guilloche":
                xs += p["warp"] * np.sin((ratio + 2.0) * t)
            elif p["warp"]:
                xs += p["warp"] * 0.15 * np.sin((ratio + 1.0) * t + phase)
                ys += p["warp"] * 0.15 * np.cos((ratio - 1.0) * t - phase)
        elif layer.family == "rose_curve":
            radius = np.cos(p["a"] / p["b"] * t + phase) * (0.72 + p["warp"] * 0.2 * np.sin(index * 0.5))
            xs, ys = radius * np.cos(t), radius * np.sin(t)
        else:  # lissajous
            shrink = 1.0 - index / max(density, 1) * 0.22
            xs = shrink * np.sin((p["a"] + index % 3) * t + phase)
            ys = shrink * np.sin((p["b"] + index % 5) * t - phase * 0.61)
            xs *= 1.0 + p["warp"] * np.sin(7 * t)
        norm = max(float(np.max(np.abs(xs))), float(np.max(np.abs(ys))), 1e-9)
        points = _points_to_pixels(xs / norm * 0.98, ys / norm * 0.98, scene.width, scene.height)
        if len(points) > 1:
            draw.line(
                points,
                fill=colors[index % len(colors)] + (45 + int(100 * (index + 1) / density),),
                width=int(p["line_width"]),
                joint="curve",
            )
    return image


def _render_moire(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    density = int(p["density"])
    base = np.linspace(-1.2, 1.2, max(scene.width, scene.height) * 2)
    offsets = np.linspace(-1.15, 1.15, density)
    cosine, sine = math.cos(p["rotation"]), math.sin(p["rotation"])
    for index, offset in enumerate(offsets):
        _check_cancel(cancel)
        wave = offset + p["warp"] * np.sin(p["frequency"] * base + index * 0.17)
        for rotate in (False, True):
            x, y = (base, wave) if not rotate else (wave, base)
            xr, yr = x * cosine - y * sine, x * sine + y * cosine
            points = _points_to_pixels(xr, yr, scene.width, scene.height, 0.0)
            draw.line(
                points,
                fill=colors[index % len(colors)] + (100,),
                width=int(p["line_width"]),
            )
    return image


def _render_orbital(
    scene: Scene, layer: Layer, seed: int, cancel: CancelCallback | None
) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    rng = random.Random(seed)
    centers = [
        (rng.uniform(-0.75, 0.75), rng.uniform(-0.75, 0.75), rng.uniform(0.11, 0.27))
        for _ in range(int(p["holes"]))
    ]
    density = int(p["density"])
    base = np.linspace(-1.15, 1.15, max(scene.width, scene.height) * 2)
    for index, offset in enumerate(np.linspace(-1.1, 1.1, density)):
        _check_cancel(cancel)
        for vertical in (False, True):
            x = base.copy() if not vertical else np.full_like(base, offset)
            y = np.full_like(base, offset) if not vertical else base.copy()
            for cx, cy, radius in centers:
                dx, dy = x - cx, y - cy
                distance = (dx * dx + dy * dy) / (radius * radius) + 0.08
                x = x + dx * p["repel"] / distance - dy * p["swirl"] / distance
                y = y + dy * p["repel"] / distance + dx * p["swirl"] / distance
            points = _points_to_pixels(x, y, scene.width, scene.height, 0.0)
            draw.line(
                points,
                fill=colors[index % len(colors)] + (92,),
                width=int(p["line_width"]),
            )
    background = _rgb(palette_stops(scene.palette)[0])
    for index, (cx, cy, radius) in enumerate(centers):
        x = (cx + 1) * scene.width / 2
        y = (1 - cy) * scene.height / 2
        r = radius * min(scene.width, scene.height) / 2
        draw.ellipse((x - r, y - r, x + r, y + r), fill=background + (255,))
        draw.ellipse(
            (x - r - 3, y - r - 3, x + r + 3, y + r + 3),
            outline=colors[index % len(colors)] + (170,),
            width=1,
        )
    return image


def _render_harmonograph(
    scene: Scene, layer: Layer, cancel: CancelCallback | None
) -> Image.Image:
    _check_cancel(cancel)
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    p = layer.params
    t = np.linspace(0.0, 85.0, int(p["samples"]))
    damp = np.exp(-p["damping"] * t)
    x = damp * (np.sin(p["frequency_x"] * t + p["phase"]) + 0.42 * np.sin((p["frequency_y"] + 0.031) * t))
    y = damp * (np.sin(p["frequency_y"] * t) + 0.42 * np.sin((p["frequency_x"] - 0.027) * t + p["phase"] * 0.4))
    norm = max(float(np.max(np.abs(x))), float(np.max(np.abs(y))), 1e-9)
    points = _points_to_pixels(x / norm, y / norm, scene.width, scene.height)
    draw.line(points, fill=_line_colors(scene.palette)[-1] + (190,), width=int(p["line_width"]), joint="curve")
    _check_cancel(cancel)
    return image


def _render_string_art(
    scene: Scene, layer: Layer, cancel: CancelCallback | None
) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    pins = int(p["pins"])
    radius = min(scene.width, scene.height) * 0.47 * p["radius"]
    cx, cy = scene.width / 2, scene.height / 2
    points = [
        (cx + radius * math.cos(math.tau * i / pins), cy - radius * math.sin(math.tau * i / pins))
        for i in range(pins)
    ]
    total = pins * int(p["cycles"])
    for index in range(total):
        if index % 256 == 0:
            _check_cancel(cancel)
        start = index % pins
        end = (index * int(p["step"])) % pins
        draw.line(
            (points[start], points[end]),
            fill=colors[index % len(colors)] + (44,),
            width=int(p["line_width"]),
        )
    return image


def _render_sierpinski(
    scene: Scene, layer: Layer, cancel: CancelCallback | None
) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    margin = min(scene.width, scene.height) * 0.05
    top = (scene.width / 2, margin)
    left = (margin, scene.height - margin)
    right = (scene.width - margin, scene.height - margin)
    stack = [(top, left, right, int(p["depth"]))]
    visited_nodes = 0
    while stack:
        visited_nodes += 1
        if visited_nodes % 256 == 0:
            _check_cancel(cancel)
        a, b, c, depth = stack.pop()
        if depth == 0:
            if p["filled"]:
                draw.polygon((a, b, c), fill=colors[0] + (120,))
            else:
                draw.line((a, b, c, a), fill=colors[depth % len(colors)] + (220,), width=int(p["line_width"]))
            continue
        ab = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
        bc = ((b[0] + c[0]) / 2, (b[1] + c[1]) / 2)
        ca = ((c[0] + a[0]) / 2, (c[1] + a[1]) / 2)
        stack.extend(((a, ab, ca, depth - 1), (ab, b, bc, depth - 1), (ca, bc, c, depth - 1)))
    return image


def _koch_segment(
    a: complex,
    b: complex,
    depth: int,
    inward: bool,
    cancel: CancelCallback | None,
) -> list[complex]:
    points = [a, b]
    sign = -1 if inward else 1
    rotation = complex(0.5, sign * math.sqrt(3) / 2)
    for _ in range(depth):
        _check_cancel(cancel)
        expanded: list[complex] = []
        for start, end in zip(points, points[1:]):
            delta = (end - start) / 3
            one = start + delta
            two = start + 2 * delta
            peak = one + delta * rotation
            expanded.extend((start, one, peak, two))
        expanded.append(points[-1])
        points = expanded
    return points


def _render_koch(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    p = layer.params
    vertices = [complex(-0.86, -0.48), complex(0.86, -0.48), complex(0.0, 0.96), complex(-0.86, -0.48)]
    points: list[complex] = []
    for a, b in zip(vertices, vertices[1:]):
        segment = _koch_segment(
            a, b, int(p["depth"]), bool(p["inward"]), cancel
        )
        points.extend(segment[:-1])
    points.append(vertices[-1])
    pixels = _points_to_pixels(np.array([z.real for z in points]), np.array([z.imag for z in points]), scene.width, scene.height)
    draw.line(pixels, fill=_line_colors(scene.palette)[-1] + (230,), width=int(p["line_width"]), joint="curve")
    return image


def _render_tree(
    scene: Scene, layer: Layer, seed: int, cancel: CancelCallback | None
) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    rng = random.Random(seed)
    start = (scene.width / 2, scene.height * 0.94)
    length = scene.height * 0.24
    jitter_val = math.radians(p["jitter"])
    branch_angle_val = math.radians(p["angle"])
    shrink_val = p["shrink"]
    max_depth = max(int(p["depth"]), 1)
    line_width = int(p["line_width"])
    stack = [(start[0], start[1], -math.pi / 2, length, max_depth)]
    visited_nodes = 0
    while stack:
        visited_nodes += 1
        if visited_nodes % 256 == 0:
            _check_cancel(cancel)
        x, y, angle, branch_length, depth = stack.pop()
        x2, y2 = x + math.cos(angle) * branch_length, y + math.sin(angle) * branch_length
        draw.line(
            (x, y, x2, y2),
            fill=colors[depth % len(colors)] + (90 + min(165, depth * 13),),
            width=max(1, int(line_width * depth / max_depth)),
        )
        if depth > 1:
            stack.append((x2, y2, angle - branch_angle_val + rng.uniform(-jitter_val, jitter_val), branch_length * shrink_val, depth - 1))
            stack.append((x2, y2, angle + branch_angle_val + rng.uniform(-jitter_val, jitter_val), branch_length * shrink_val, depth - 1))
    return image


def _render_dragon(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    turns = [1]
    for _ in range(int(layer.params["depth"]) - 1):
        _check_cancel(cancel)
        rev = [-turn for turn in reversed(turns)]
        turns.append(1)
        turns.extend(rev)
    direction = complex(1, 0)
    point = complex(0, 0)
    points = [point]
    for index, turn in enumerate(turns):
        if index % 10_000 == 0:
            _check_cancel(cancel)
        point += direction
        points.append(point)
        direction *= complex(0, turn)
    values = np.asarray(points)
    angle = layer.params["rotation"]
    values *= complex(math.cos(angle), math.sin(angle))
    xr, yi = values.real, values.imag
    xr = (xr - (xr.min() + xr.max()) / 2) / max((xr.max() - xr.min()) / 2, 1e-9)
    yi = (yi - (yi.min() + yi.max()) / 2) / max((yi.max() - yi.min()) / 2, 1e-9)
    image = _transparent(scene.width, scene.height)
    ImageDraw.Draw(image, "RGBA").line(
        _points_to_pixels(xr, yi, scene.width, scene.height),
        fill=_line_colors(scene.palette)[-1] + (220,),
        width=int(layer.params["line_width"]),
        joint="curve",
    )
    return image


def _render_fern(scene: Scene, layer: Layer, seed: int, cancel: CancelCallback | None) -> Image.Image:
    p = layer.params
    count = int(p["points"])
    rng = random.Random(seed)
    xs = np.empty(count, dtype=np.float64)
    ys = np.empty(count, dtype=np.float64)
    x = y = 0.0
    for index in range(count):
        if index % 10_000 == 0:
            _check_cancel(cancel)
        value = rng.random()
        if value < 0.01:
            x, y = 0.0, 0.16 * y
        elif value < 0.86:
            x, y = 0.85 * x + 0.04 * y, -0.04 * x + 0.85 * y + 1.6
        elif value < 0.93:
            x, y = 0.2 * x - 0.26 * y, 0.23 * x + 0.22 * y + 1.6
        else:
            x, y = -0.15 * x + 0.28 * y, 0.26 * x + 0.24 * y + 0.44
        xs[index], ys[index] = x, y
    px = np.clip(((xs * p["spread"] + 3.0) / 6.0 * (scene.width - 1)).astype(int), 0, scene.width - 1)
    py = np.clip(((10.2 - ys) / 10.2 * (scene.height - 1)).astype(int), 0, scene.height - 1)
    density = np.zeros((scene.height, scene.width), dtype=np.float64)
    np.add.at(density, (py, px), 1.0)
    density = np.log1p(density)
    if density.max() > 0:
        density /= density.max()
    image = _scalar_image(density**0.45, scene.palette, density > 0)
    dot_size = int(p["dot_size"])
    if dot_size > 1:
        image = image.filter(ImageFilter.MaxFilter(dot_size * 2 - 1))
    return image


def _render_l_system(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    p = layer.params
    preset = p["preset"]
    definitions = {
        "plant": ("X", {"X": "F+[[X]-X]-F[-FX]+X", "F": "FF"}, p["angle"], {"F"}),
        "koch": ("F--F--F", {"F": "F+F--F+F"}, 60.0, {"F"}),
        "hilbert": ("A", {"A": "-BF+AFA+FB-", "B": "+AF-BFB-FA+"}, 90.0, {"F"}),
        "gosper": ("A", {"A": "A-B--B+A++AA+B-", "B": "+A-BB--B-A++A+B"}, 60.0, {"A", "B"}),
    }
    symbols, rules, _canonical_angle, draw_symbols = definitions[preset]
    angle_degrees = float(p["angle"])
    for _ in range(int(p["depth"])):
        _check_cancel(cancel)
        symbols = "".join(rules.get(symbol, symbol) for symbol in symbols)
        if len(symbols) > 400_000:
            raise SceneError("L-system expansion exceeds the 400,000-symbol safety budget")
    angle_step = math.radians(angle_degrees)
    heading = -math.pi / 2 if preset == "plant" else 0.0
    position = complex(0.0, 0.0)
    stack: list[tuple[complex, float]] = []
    segments: list[tuple[complex, complex]] = []
    for index, symbol in enumerate(symbols):
        if index % 20_000 == 0:
            _check_cancel(cancel)
        if symbol in draw_symbols:
            next_position = position + complex(math.cos(heading), math.sin(heading))
            segments.append((position, next_position))
            position = next_position
        elif symbol == "+":
            heading += angle_step
        elif symbol == "-":
            heading -= angle_step
        elif symbol == "[":
            stack.append((position, heading))
        elif symbol == "]" and stack:
            position, heading = stack.pop()
    if not segments:
        raise SceneError("L-system produced no drawable segments")
    all_x = np.asarray([point.real for segment in segments for point in segment])
    all_y = np.asarray([point.imag for segment in segments for point in segment])
    center_x, center_y = (all_x.min() + all_x.max()) / 2, (all_y.min() + all_y.max()) / 2
    extent = max((all_x.max() - all_x.min()) / 2, (all_y.max() - all_y.min()) / 2, 1e-9)
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    for index, (start, end) in enumerate(segments):
        xs = np.asarray([(start.real - center_x) / extent, (end.real - center_x) / extent])
        ys = np.asarray([(start.imag - center_y) / extent, (end.imag - center_y) / extent])
        points = _points_to_pixels(xs, ys, scene.width, scene.height)
        draw.line(
            points,
            fill=colors[(index * len(colors) // max(len(segments), 1)) % len(colors)] + (210,),
            width=int(p["line_width"]),
        )
    return image


def _render_truchet(
    scene: Scene, layer: Layer, seed: int, cancel: CancelCallback | None
) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    rng = random.Random(seed)
    tiles = int(p["tiles"])
    size = max(scene.width, scene.height) / tiles
    rows = math.ceil(scene.height / size)
    columns = math.ceil(scene.width / size)
    for row in range(rows):
        _check_cancel(cancel)
        for column in range(columns):
            x, y = column * size, row * size
            flipped = rng.random() < p["rotation_bias"]
            color = colors[(row + column) % len(colors)] + (205,)
            boxes = (
                ((x - size, y - size, x + size, y + size), (x, y, x + 2 * size, y + 2 * size))
                if not flipped
                else ((x, y - size, x + 2 * size, y + size), (x - size, y, x + size, y + 2 * size))
            )
            for box in boxes:
                draw.arc(box, 0, 360, fill=color, width=int(p["line_width"]))
                if p["double"]:
                    inset = min(float(p["line_width"]) * 1.5, size * 0.18)
                    inner = (box[0] + inset, box[1] + inset, box[2] - inset, box[3] - inset)
                    if inner[2] >= inner[0] and inner[3] >= inner[1]:
                        draw.arc(inner, 0, 360, fill=color, width=1)
    return image


def _render_hex(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    columns = int(p["columns"])
    radius = scene.width / columns / math.sqrt(3)
    row_step = radius * 1.5
    rows = int(scene.height / row_step) + 3
    for row in range(-1, rows):
        _check_cancel(cancel)
        for column in range(-1, columns + 2):
            cx = column * radius * math.sqrt(3) + (row % 2) * radius * math.sqrt(3) / 2
            cy = row * row_step
            for ring in range(1, int(p["rings"]) + 1):
                rr = radius * ring / int(p["rings"])
                points = [
                    (
                        cx + rr * math.cos(p["twist"] + math.pi / 3 * index),
                        cy + rr * math.sin(p["twist"] + math.pi / 3 * index),
                    )
                    for index in range(6)
                ]
                points.append(points[0])
                draw.line(points, fill=colors[(row + column + ring) % len(colors)] + (125,), width=int(p["line_width"]))
    return image


def _render_op_art(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    base = np.linspace(-1.2, 1.2, max(scene.width, scene.height) * 2)
    cosine, sine = math.cos(p["rotation"]), math.sin(p["rotation"])
    for index, offset in enumerate(np.linspace(-1.1, 1.1, int(p["density"]))):
        _check_cancel(cancel)
        wave = offset + p["warp"] * np.sin(p["frequency"] * base + offset * 4.0) * np.cos(base * 1.7)
        x = base * cosine - wave * sine
        y = base * sine + wave * cosine
        draw.line(
            _points_to_pixels(x, y, scene.width, scene.height, 0.0),
            fill=colors[index % len(colors)] + (145,),
            width=int(p["line_width"]),
        )
    return image


def _render_cellular(
    scene: Scene, layer: Layer, seed: int, cancel: CancelCallback | None
) -> Image.Image:
    p = layer.params
    rows = int(p["rows"])
    columns = max(64, scene.width)
    state = np.zeros(columns, dtype=np.uint8)
    if p["random_start"]:
        rng = np.random.default_rng(seed)
        state = rng.integers(0, 2, columns, dtype=np.uint8)
    else:
        state[columns // 2] = 1
    field = np.zeros((rows, columns), dtype=np.uint8)
    rule = int(p["rule"])
    for row in range(rows):
        if row % 64 == 0:
            _check_cancel(cancel)
        field[row] = state
        left, right = np.roll(state, 1), np.roll(state, -1)
        index = (left << 2) | (state << 1) | right
        state = ((rule >> index) & 1).astype(np.uint8)
        if p["mirror"]:
            state = np.maximum(state, state[::-1])
    image = _scalar_image(field.astype(float), scene.palette, field > 0)
    return image.resize((scene.width, scene.height), Image.Resampling.NEAREST)


def _render_reaction(
    scene: Scene,
    layer: Layer,
    seed: int,
    progress: ProgressCallback | None,
    cancel: CancelCallback | None,
) -> Image.Image:
    p = layer.params
    size = max(64, min(192, min(scene.width, scene.height)))
    a = np.ones((size, size), dtype=np.float64)
    b = np.zeros_like(a)
    rng = random.Random(seed)
    for _ in range(int(p["spots"])):
        x, y = rng.randrange(4, size - 4), rng.randrange(4, size - 4)
        radius = rng.randrange(2, max(3, size // 18))
        b[max(0, y - radius) : y + radius, max(0, x - radius) : x + radius] = rng.uniform(0.65, 1.0)
    iterations = int(p["iterations"])
    for iteration in range(iterations):
        if iteration % 10 == 0:
            _check_cancel(cancel)
            _notify(progress, iteration / iterations, f"reaction diffusion: {iteration}/{iterations}")
        lap_a = -a + 0.2 * (np.roll(a, 1, 0) + np.roll(a, -1, 0) + np.roll(a, 1, 1) + np.roll(a, -1, 1)) + 0.05 * (np.roll(np.roll(a, 1, 0), 1, 1) + np.roll(np.roll(a, 1, 0), -1, 1) + np.roll(np.roll(a, -1, 0), 1, 1) + np.roll(np.roll(a, -1, 0), -1, 1))
        lap_b = -b + 0.2 * (np.roll(b, 1, 0) + np.roll(b, -1, 0) + np.roll(b, 1, 1) + np.roll(b, -1, 1)) + 0.05 * (np.roll(np.roll(b, 1, 0), 1, 1) + np.roll(np.roll(b, 1, 0), -1, 1) + np.roll(np.roll(b, -1, 0), 1, 1) + np.roll(np.roll(b, -1, 0), -1, 1))
        reaction = a * b * b
        a += p["diff_a"] * lap_a - reaction + p["feed"] * (1.0 - a)
        b += p["diff_b"] * lap_b + reaction - (p["kill"] + p["feed"]) * b
        np.clip(a, 0.0, 1.0, out=a)
        np.clip(b, 0.0, 1.0, out=b)
    values = np.clip((a - b + 0.35) / 1.35, 0.0, 1.0)
    image = _scalar_image(values, scene.palette)
    return image.resize((scene.width, scene.height), Image.Resampling.BICUBIC)


def _render_flow(scene: Scene, layer: Layer, seed: int, cancel: CancelCallback | None) -> Image.Image:
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    colors = _line_colors(scene.palette)
    p = layer.params
    rng = random.Random(seed)
    streams = int(p["streams"])
    freq = float(p["frequency"])
    curl = float(p["curl"])
    step_size = float(p["step_size"])
    steps = int(p["steps"])
    line_width = int(p["line_width"])
    for stream in range(streams):
        if stream % 100 == 0:
            _check_cancel(cancel)
        x, y = rng.uniform(-1.0, 1.0), rng.uniform(-1.0, 1.0)
        points_x, points_y = [x], [y]
        for _ in range(steps):
            angle = freq * math.sin(y * math.pi) + curl * math.cos(x * math.pi) + 0.5 * math.sin((x + y) * math.pi)
            x += math.cos(angle) * step_size
            y += math.sin(angle) * step_size
            if not (-1.1 <= x <= 1.1 and -1.1 <= y <= 1.1):
                break
            points_x.append(x)
            points_y.append(y)
        if len(points_x) > 1:
            draw.line(
                _points_to_pixels(np.asarray(points_x), np.asarray(points_y), scene.width, scene.height, 0.0),
                fill=colors[stream % len(colors)] + (55,),
                width=line_width,
            )
    return image


def _render_attractor(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    p = layer.params
    count = int(p["points"])
    xs, ys = np.empty(count), np.empty(count)
    x = y = 0.1
    a = float(p["a"])
    b = float(p["b"])
    c = float(p["c"])
    d = float(p["d"])
    for index in range(count + 100):
        if index % 20_000 == 0:
            _check_cancel(cancel)
        x, y = math.sin(a * y) + c * math.cos(a * x), math.sin(b * x) + d * math.cos(b * y)
        if index >= 100:
            xs[index - 100], ys[index - 100] = x, y
    x_min, x_max = np.percentile(xs, [0.2, 99.8])
    y_min, y_max = np.percentile(ys, [0.2, 99.8])
    px = np.clip(((xs - x_min) / max(x_max - x_min, 1e-9) * (scene.width - 1)).astype(int), 0, scene.width - 1)
    py = np.clip(((y_max - ys) / max(y_max - y_min, 1e-9) * (scene.height - 1)).astype(int), 0, scene.height - 1)
    density = np.zeros((scene.height, scene.width), dtype=np.float64)
    np.add.at(density, (py, px), 1.0)
    density = np.log1p(density)
    if density.max() > 0:
        density /= density.max()
    image = _scalar_image(density**0.42, scene.palette, density > 0)
    dot_size = int(p["dot_size"])
    if dot_size > 1:
        image = image.filter(ImageFilter.MaxFilter(dot_size * 2 - 1))
    return image


def _render_layer(
    scene: Scene,
    layer: Layer,
    layer_index: int,
    progress: ProgressCallback | None,
    cancel: CancelCallback | None,
) -> Image.Image:
    renderer = FAMILY_SPECS[layer.family].renderer
    seed = _mixed_seed(scene.seed, layer_index, layer.family)
    if renderer == "escape":
        return _render_escape(scene, layer, progress, cancel)
    if renderer == "newton":
        return _render_newton(scene, layer, progress, cancel)
    if renderer == "polar":
        return _render_polar(scene, layer, cancel)
    if renderer == "star_mandala":
        return _render_star_mandala(scene, layer, cancel)
    if renderer == "flower":
        return _render_flower(scene, layer, cancel)
    if renderer == "phyllotaxis":
        return _render_phyllotaxis(scene, layer, seed, cancel)
    if renderer == "curves":
        return _render_curves(scene, layer, cancel)
    if renderer == "moire":
        return _render_moire(scene, layer, cancel)
    if renderer == "orbital":
        return _render_orbital(scene, layer, seed, cancel)
    if renderer == "harmonograph":
        return _render_harmonograph(scene, layer, cancel)
    if renderer == "string_art":
        return _render_string_art(scene, layer, cancel)
    if renderer == "sierpinski":
        return _render_sierpinski(scene, layer, cancel)
    if renderer == "koch":
        return _render_koch(scene, layer, cancel)
    if renderer == "tree":
        return _render_tree(scene, layer, seed, cancel)
    if renderer == "dragon":
        return _render_dragon(scene, layer, cancel)
    if renderer == "fern":
        return _render_fern(scene, layer, seed, cancel)
    if renderer == "l_system":
        return _render_l_system(scene, layer, cancel)
    if renderer == "truchet":
        return _render_truchet(scene, layer, seed, cancel)
    if renderer == "hex":
        return _render_hex(scene, layer, cancel)
    if renderer == "op_art":
        return _render_op_art(scene, layer, cancel)
    if renderer == "cellular":
        return _render_cellular(scene, layer, seed, cancel)
    if renderer == "reaction":
        return _render_reaction(scene, layer, seed, progress, cancel)
    if renderer == "flow":
        return _render_flow(scene, layer, seed, cancel)
    if renderer == "attractor":
        return _render_attractor(scene, layer, cancel)
    if renderer == "particles":
        return _render_particles(scene, layer, cancel)
    raise SceneError(f"family '{layer.family}' has no renderer")


def _composite(base: Image.Image, layer: Image.Image, alpha: float, blend: str) -> Image.Image:
    layer = layer.copy()
    if alpha < 1.0:
        layer.putalpha(layer.getchannel("A").point(lambda value: int(value * alpha)))
    if blend == "normal":
        return Image.alpha_composite(base, layer)
    base_rgb, layer_rgb = base.convert("RGB"), layer.convert("RGB")
    if blend == "screen":
        mixed = ImageChops.screen(base_rgb, layer_rgb)
    elif blend == "multiply":
        mixed = ImageChops.multiply(base_rgb, layer_rgb)
    else:
        mixed = ImageChops.add(base_rgb, layer_rgb, scale=1.0, offset=0)
    candidate = Image.merge("RGBA", (*mixed.split(), Image.new("L", base.size, 255)))
    return Image.composite(candidate, base, layer.getchannel("A"))


def _quality_metrics(image: Image.Image, background: tuple[int, int, int]) -> dict[str, Any]:
    # 1. Convert to RGB once
    rgb_image = image.convert("RGB")
    
    # 2. Use zero-copy uint8 view for luminance
    array = np.asarray(rgb_image)
    luminance = array[..., 0] * 0.2126 + array[..., 1] * 0.7152 + array[..., 2] * 0.0722
    
    # 3. Calculate difference entirely in uint8 using np.maximum and np.minimum
    bg_arr = np.asarray(background, dtype=np.uint8)
    d0 = np.maximum(array[..., 0], bg_arr[0]) - np.minimum(array[..., 0], bg_arr[0])
    d1 = np.maximum(array[..., 1], bg_arr[1]) - np.minimum(array[..., 1], bg_arr[1])
    d2 = np.maximum(array[..., 2], bg_arr[2]) - np.minimum(array[..., 2], bg_arr[2])
    difference = np.maximum(np.maximum(d0, d1), d2)
    
    # 4. Use PIL getcolors on the resized sample (max 4096 colors for 64x64)
    sample = rgb_image.resize((64, 64), Image.Resampling.BILINEAR)
    colors = sample.getcolors(maxcolors=4096)
    unique_colors = len(colors) if colors is not None else 0
    
    return {
        "luminance_mean": round(float(luminance.mean()), 4),
        "luminance_std": round(float(luminance.std()), 4),
        "non_background_fraction": round(float(np.mean(difference > 3)), 6),
        "sample_unique_colors": unique_colors,
        "nan_count": 0,
    }


def verify_render_result(result: RenderResult) -> None:
    if result.image.mode != "RGBA":
        raise SceneError("renderer did not produce RGBA output")
    if result.metrics["non_background_fraction"] < 0.0005:
        raise SceneError("render is effectively blank")
    if result.metrics["sample_unique_colors"] < 2:
        raise SceneError("render contains no visible variation")
    if result.metrics["nan_count"]:
        raise SceneError("render contains non-finite values")


def render_scene(
    scene: Scene,
    frame_index: int = 0,
    fps: float = 30.0,
    progress: ProgressCallback | None = None,
    cancel: CancelCallback | None = None,
) -> RenderResult:
    started = time.perf_counter()
    scene = validate_scene(scene)
    effective = validate_scene(scene_for_frame(scene, frame_index, fps))
    background = _rgb(effective.background or palette_stops(effective.palette)[0])
    canvas = Image.new("RGBA", (effective.width, effective.height), background + (255,))
    visible_layers = [(index, item) for index, item in enumerate(effective.layers) if item.visible]
    if not visible_layers:
        raise SceneError("a scene must have at least one visible layer")
    for position, (index, layer) in enumerate(visible_layers):
        _check_cancel(cancel)

        def layer_progress(value: float, stage: str) -> None:
            _notify(progress, (position + value) / len(visible_layers), stage)

        rendered = _render_layer(effective, layer, index, layer_progress, cancel)
        _check_cancel(cancel)
        canvas = _composite(canvas, rendered, layer.alpha, layer.blend)
        _notify(progress, (position + 1) / len(visible_layers), f"composited {layer.family}")
    raw = canvas.tobytes()
    result = RenderResult(
        image=canvas,
        recipe_hash=scene.recipe_hash(),
        render_hash=hashlib.sha256(raw).hexdigest(),
        metrics=_quality_metrics(canvas, background),
        duration_ms=int((time.perf_counter() - started) * 1000),
        frame_index=int(frame_index),
    )
    verify_render_result(result)
    return result


def save_render_result(
    result: RenderResult,
    scene: Scene,
    path: str | os.PathLike[str],
    write_manifest: bool = True,
) -> Path:
    """Save with extension-correct format and propagate every error to the caller."""

    target = Path(path).expanduser()
    if not target.suffix:
        target = target.with_suffix(".png")
    suffix = target.suffix.lower()
    formats = {".png": "PNG", ".jpg": "JPEG", ".jpeg": "JPEG", ".webp": "WEBP"}
    if suffix not in formats:
        raise SceneError("output format must be PNG, JPEG, or WEBP")
    target.parent.mkdir(parents=True, exist_ok=True)
    transaction = f"{os.getpid()}.{uuid.uuid4().hex}"
    temporary = target.with_name(
        f".{target.stem}.{transaction}.tmp{target.suffix}"
    )
    image = result.image.convert("RGB") if formats[suffix] == "JPEG" else result.image
    manifest_path = target.with_suffix(target.suffix + ".json")
    manifest_tmp = manifest_path.with_name(
        f".{manifest_path.name}.{transaction}.tmp"
    )
    image_backup = target.with_name(f".{target.name}.{transaction}.bak")
    manifest_backup = manifest_path.with_name(
        f".{manifest_path.name}.{transaction}.bak"
    )
    committed_image = False
    committed_manifest = False
    try:
        image.save(temporary, format=formats[suffix], quality=95)
        if write_manifest:
            manifest = result.manifest(scene)
            manifest["output_path"] = str(target.resolve())
            manifest_tmp.write_text(
                json.dumps(manifest, indent=2, sort_keys=True, allow_nan=False) + "\n",
                encoding="utf-8",
            )
        if target.exists():
            os.replace(target, image_backup)
        if write_manifest and manifest_path.exists():
            os.replace(manifest_path, manifest_backup)
        os.replace(temporary, target)
        committed_image = True
        if write_manifest:
            os.replace(manifest_tmp, manifest_path)
            committed_manifest = True
        image_backup.unlink(missing_ok=True)
        manifest_backup.unlink(missing_ok=True)
    except Exception:
        # The manifest is the commit marker. Roll back the image whenever the
        # pair cannot be published completely, restoring any previous pair.
        if committed_manifest:
            manifest_path.unlink(missing_ok=True)
        if committed_image:
            target.unlink(missing_ok=True)
        if image_backup.exists():
            os.replace(image_backup, target)
        if manifest_backup.exists():
            os.replace(manifest_backup, manifest_path)
        temporary.unlink(missing_ok=True)
        manifest_tmp.unlink(missing_ok=True)
        raise
    return target


# === Animation support (Fractus upgrade) ===
# The DSL already parses "animate" commands. This adds actual GIF output
# so agents (Fractus, court) can produce animated images.
# Uses PIL (already a dep). For 3D-ish green particle feel, see new "particles" family stub.

def render_animated_gif(
    scene: Scene,
    num_frames: int = 24,
    fps: int = 12,
    cancel: CancelCallback | None = None,
    progress: ProgressCallback | None = None,
    output_path: str | os.PathLike[str] | None = None,
) -> Path:
    """Render a deterministic animation cycle and atomically publish a GIF."""
    scene = validate_scene(scene)
    if not scene.animations:
        raise SceneError("animated GIF output requires at least one animate line")
    num_frames = max(2, min(int(num_frames), 180))
    fps = max(1, min(int(fps), 30))
    cycle_seconds = min(max(item.seconds for item in scene.animations), 20.0)
    frames: list[Image.Image] = []
    for i in range(num_frames):
        if cancel and cancel():
            raise RenderCancelled("animation cancelled")
        frame_index = round(i * cycle_seconds * fps / (num_frames - 1))
        res = render_scene(scene, frame_index=frame_index, fps=fps, cancel=cancel)
        frames.append(res.image.convert("P", palette=Image.ADAPTIVE))
        if progress:
            progress((i + 1) / num_frames, f"frame {i+1}/{num_frames}")

    safe_name = "".join(char if char.isalnum() or char in "-_" else "_" for char in (scene.name or "animated"))
    out_path = (
        Path(output_path).expanduser()
        if output_path is not None
        else Path(__file__).resolve().parent / "output" / f"{safe_name}_{scene.recipe_hash()[:8]}_anim.gif"
    )
    out_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = out_path.with_name(f".{out_path.stem}.{os.getpid()}.tmp.gif")
    frames[0].save(
        temporary,
        format="GIF",
        save_all=True,
        append_images=frames[1:],
        duration=int(1000 / fps),
        loop=0,
        optimize=True,
    )
    os.replace(temporary, out_path)
    return out_path


def _render_particles(scene: Scene, layer: Layer, cancel: CancelCallback | None) -> Image.Image:
    """Simple particle system with pseudo-3D projection (depth, perspective, rotation).
    Supports animation via 'phase' param (e.g. animate 0.phase from=0 to=6.28).
    Good for green particle 3D-ish animated effects.
    """
    image = _transparent(scene.width, scene.height)
    draw = ImageDraw.Draw(image, "RGBA")
    p = layer.params
    rng = random.Random(scene.seed)
    count = int(p["count"])
    speed = float(p["speed"])
    size = float(p["size"])
    depth = float(p["depth"])
    rot = float(p["rotation"])
    phase = float(p.get("phase", 0.0))
    hue = float(p.get("hue_shift", 0.0))

    cx, cy = scene.width * 0.5, scene.height * 0.5
    scale = min(scene.width, scene.height) * 0.4

    for i in range(count):
        if cancel and cancel():
            break
        # base pseudo 3D position
        z = rng.uniform(0.2, depth)
        x = rng.uniform(-1.0, 1.0) * z
        y = rng.uniform(-1.0, 1.0) * z

        # animate: move particles along a flow using phase and speed (gives life/motion)
        t = phase * speed
        flow = 0.3 * math.sin(i * 0.7 + t)   # individual wiggle
        x += flow * (1 + (i % 3) * 0.2)
        y += flow * 0.8 * math.cos(i * 1.1 + t * 0.7)

        # rotation around center (can be animated too)
        ca = math.cos(rot + t * 0.1)  # slight auto-rotate with time
        sa = math.sin(rot + t * 0.1)
        rx = x * ca - y * sa
        ry = x * sa + y * ca

        # perspective projection for 3D look
        persp = 1.0 / (z + 0.5)
        px = cx + (rx * persp) * scale
        py = cy + (ry * persp) * scale

        # size falloff with depth + slight pulse for animation
        r = max(0.5, size * persp * (1.0 + 0.2 * math.sin(t + i)))

        # color: prefer green-ish for the "green particle" by shifting palette if emerald-like
        colors = _line_colors(scene.palette)
        color = colors[i % len(colors)]
        alpha = int(140 + 110 * persp)

        draw.ellipse([px-r, py-r, px+r, py+r], fill=color + (alpha,))

    return image

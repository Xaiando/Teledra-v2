#!/usr/bin/env python3
"""Stream locked safetensors one tensor at a time and emit truthful delta stats."""

from __future__ import annotations

import argparse
from contextlib import ExitStack
import json
import math
import os
from pathlib import Path
import re
import tempfile

from safetensors import safe_open


CHUNK = 1_048_576


def tensor_map(root: Path) -> dict[str, Path]:
    root = root.resolve(strict=True)
    indexes = sorted(root.glob("*.safetensors.index.json"))
    mapping: dict[str, Path] = {}
    if indexes:
        if len(indexes) != 1:
            raise ValueError(f"expected one safetensors index in {root}")
        payload = json.loads(indexes[0].read_text(encoding="utf-8"))
        for name, relative in payload.get("weight_map", {}).items():
            path = (root / str(relative)).resolve(strict=True)
            if root not in path.parents or path.suffix != ".safetensors":
                raise ValueError(f"unsafe shard path for {name}: {path}")
            mapping[str(name)] = path
    else:
        for path in sorted(root.glob("*.safetensors")):
            with safe_open(str(path), framework="pt", device="cpu") as handle:
                for name in handle.keys():
                    if name in mapping:
                        raise ValueError(f"duplicate tensor {name}")
                    mapping[name] = path.resolve()
    if not mapping:
        raise ValueError(f"no safetensors weights found in {root}")
    return mapping


def layer_name(tensor: str) -> str:
    found = re.search(r"(?:layers|blocks|h)\.(\d+)", tensor)
    if found:
        return f"layer_{int(found.group(1)):04d}"
    return ".".join(tensor.split(".")[:2])


def finite(value: float) -> float:
    return value if math.isfinite(value) else 0.0


def inspect_tensor(name: str, files: list[Path], donor_names: list[str]) -> tuple[dict, list[str]]:
    reasons: list[str] = []
    with ExitStack() as stack:
        handles = [stack.enter_context(safe_open(str(path), framework="pt", device="cpu")) for path in files]
        tensors = [handle.get_tensor(name) for handle in handles]
        shapes = [list(tensor.shape) for tensor in tensors]
        dtypes = [str(tensor.dtype).replace("torch.", "") for tensor in tensors]
        if any(shape != shapes[0] for shape in shapes[1:]):
            return {"tensor": name, "shapes": shapes, "dtypes": dtypes}, [f"shape mismatch: {name}"]
        flats = [tensor.reshape(-1) for tensor in tensors]
        base_sq = 0.0
        donor_sq = [0.0] * len(donor_names)
        pair_dot: dict[str, float] = {}
        pair_compared: dict[str, int] = {}
        pair_conflicts: dict[str, int] = {}
        for start in range(0, flats[0].numel(), CHUNK):
            end = min(flats[0].numel(), start + CHUNK)
            base = flats[0][start:end].float()
            deltas = [flat[start:end].float() - base for flat in flats[1:]]
            base_sq += float((base * base).sum().item())
            for index, delta in enumerate(deltas):
                donor_sq[index] += float((delta * delta).sum().item())
            for left in range(len(deltas)):
                for right in range(left + 1, len(deltas)):
                    key = f"{donor_names[left]}::{donor_names[right]}"
                    pair_dot[key] = pair_dot.get(key, 0.0) + float((deltas[left] * deltas[right]).sum().item())
                    nonzero = (deltas[left] != 0) & (deltas[right] != 0)
                    pair_compared[key] = pair_compared.get(key, 0) + int(nonzero.sum().item())
                    pair_conflicts[key] = pair_conflicts.get(key, 0) + int(((deltas[left] * deltas[right] < 0) & nonzero).sum().item())
        donor_stats = {}
        for index, donor in enumerate(donor_names):
            norm = math.sqrt(max(0.0, donor_sq[index]))
            donor_stats[donor] = {
                "delta_l2": finite(norm),
                "base_l2": finite(math.sqrt(max(0.0, base_sq))),
                "normalized_delta": finite(norm / max(math.sqrt(max(0.0, base_sq)), 1e-12)),
            }
        pairs = {}
        for key, dot in pair_dot.items():
            left, right = key.split("::", 1)
            left_norm = math.sqrt(max(0.0, donor_sq[donor_names.index(left)]))
            right_norm = math.sqrt(max(0.0, donor_sq[donor_names.index(right)]))
            compared = pair_compared.get(key, 0)
            pairs[key] = {
                "cosine": finite(dot / max(left_norm * right_norm, 1e-12)),
                "sign_conflict_rate": finite(pair_conflicts.get(key, 0) / max(compared, 1)),
                "compared_nonzero": compared,
            }
        return {
            "tensor": name,
            "layer": layer_name(name),
            "shape": shapes[0],
            "dtype": dtypes[0],
            "elements": flats[0].numel(),
            "donors": donor_stats,
            "pairs": pairs,
        }, reasons


def analyze(base: Path, donors: list[tuple[str, Path]], output: Path, summary_path: Path) -> dict:
    base_map = tensor_map(base)
    donor_maps = [(name, tensor_map(path)) for name, path in donors]
    reasons: list[str] = []
    base_names = set(base_map)
    for donor, mapping in donor_maps:
        missing = sorted(base_names - set(mapping))
        extra = sorted(set(mapping) - base_names)
        if missing:
            reasons.append(f"{donor} missing {len(missing)} tensor(s), first={missing[0]}")
        if extra:
            reasons.append(f"{donor} has {len(extra)} extra tensor(s), first={extra[0]}")
    names = sorted(base_names.intersection(*(set(mapping) for _, mapping in donor_maps)))
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(output.suffix + ".tmp")
    ranks: list[tuple[float, str]] = []
    conflicts: list[tuple[float, str]] = []
    layers: dict[str, dict[str, dict[str, float]]] = {}
    with temporary.open("w", encoding="utf-8", newline="\n") as stream:
        for index, name in enumerate(names, 1):
            files = [base_map[name], *(mapping[name] for _, mapping in donor_maps)]
            row, row_reasons = inspect_tensor(name, files, [donor for donor, _ in donor_maps])
            reasons.extend(row_reasons)
            stream.write(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n")
            maximum = max((value["normalized_delta"] for value in row.get("donors", {}).values()), default=0.0)
            conflict = max((value["sign_conflict_rate"] for value in row.get("pairs", {}).values()), default=0.0)
            ranks.append((maximum, name))
            conflicts.append((conflict, name))
            layer = row.get("layer")
            if layer:
                aggregate = layers.setdefault(layer, {})
                for donor, values in row.get("donors", {}).items():
                    target = aggregate.setdefault(donor, {"delta_l2_sq": 0.0, "base_l2_sq": 0.0, "tensors": 0})
                    target["delta_l2_sq"] += values["delta_l2"] ** 2
                    target["base_l2_sq"] += values["base_l2"] ** 2
                    target["tensors"] += 1
            if index % 100 == 0:
                print(f"inspected {index}/{len(names)} tensors", flush=True)
    os.replace(temporary, output)
    layer_summary = {}
    for layer, donor_values in layers.items():
        layer_summary[layer] = {}
        for donor, values in donor_values.items():
            delta = math.sqrt(values.pop("delta_l2_sq"))
            base_norm = math.sqrt(values.pop("base_l2_sq"))
            layer_summary[layer][donor] = {
                "delta_l2": finite(delta),
                "normalized_delta": finite(delta / max(base_norm, 1e-12)),
                "tensors": int(values["tensors"]),
            }
    summary = {
        "compatible": not reasons,
        "tensors": len(names),
        "donors": len(donors),
        "reasons": reasons[:100],
        "top_normalized_delta": [{"tensor": name, "value": value} for value, name in sorted(ranks, reverse=True)[:50]],
        "top_sign_conflict": [{"tensor": name, "value": value} for value, name in sorted(conflicts, reverse=True)[:50]],
        "layers": layer_summary,
    }
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    temp_summary = summary_path.with_suffix(summary_path.suffix + ".tmp")
    temp_summary.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temp_summary, summary_path)
    return summary


def self_test() -> None:
    import torch
    from safetensors.torch import save_file

    with tempfile.TemporaryDirectory(prefix="frankenswarm-tensor-test-") as raw:
        root = Path(raw)
        base, first, second = root / "base", root / "first", root / "second"
        for path in (base, first, second):
            path.mkdir()
        original = {"model.layers.0.weight": torch.tensor([[1.0, -2.0], [3.0, 4.0]])}
        save_file(original, base / "model.safetensors")
        save_file({key: value + 0.5 for key, value in original.items()}, first / "model.safetensors")
        save_file({key: value - 0.25 for key, value in original.items()}, second / "model.safetensors")
        summary = analyze(base, [("first", first), ("second", second)], root / "stats.jsonl", root / "summary.json")
        assert summary["compatible"] and summary["tensors"] == 1 and summary["donors"] == 2
        row = json.loads((root / "stats.jsonl").read_text(encoding="utf-8"))
        assert row["pairs"]["first::second"]["sign_conflict_rate"] == 1.0
    print("tensor_autopsy self-test passed")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", type=Path)
    parser.add_argument("--donor", action="append", default=[])
    parser.add_argument("--output", type=Path)
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not args.base or not args.output or not args.summary or not args.donor:
        parser.error("--base, --donor, --output, and --summary are required")
    donors = []
    for raw in args.donor:
        if "=" not in raw:
            parser.error("--donor must be NAME=PATH")
        name, path = raw.split("=", 1)
        if not re.fullmatch(r"[A-Za-z0-9_.-]{1,100}", name):
            parser.error("unsafe donor name")
        donors.append((name, Path(path)))
    if len({name for name, _ in donors}) != len(donors):
        parser.error("duplicate donor name")
    summary = analyze(args.base, donors, args.output, args.summary)
    print(json.dumps({key: summary[key] for key in ("compatible", "tensors", "donors", "reasons")}))


if __name__ == "__main__":
    main()

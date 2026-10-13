"""Blend the retro and traditional songbooks into one theory corpus.

Buzzart learns naturalness from a single songbook, and the three we have were
built separately: chiptune (console), tracker modules (amiga), and folk plus
Bach chorales (songbook). Training on one means the others are unheard.

Pooling raw material would not work. The traditional corpus holds 264,186
notes against the chiptune corpus's 2,461 — a hundred to one — so a naive
merge is a folk songbook with a rounding error of arcade in it. Every table in
these files is already normalised to a probability distribution, which is what
makes a deliberate blend possible: the weights below decide how much of each
tradition Buzzart hears, and corpus size decides nothing.

The retro half leads because that is the sound being asked for; the
traditional half is what gives it somewhere to go, since 2,461 notes of
chiptune cannot teach voice leading on their own.

    ..\\.venv\\Scripts\\python.exe -m flycomposer.buzzart.blend_build
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "data" / "foundation"

# Retro-led: 65% of what Buzzart hears is game and demoscene music, 35% is the
# folk and chorale corpus that carries the voice leading and cadences.
DEFAULT_WEIGHTS = {"console": .40, "amiga": .25, "songbook": .35}

# Enough motifs for variety without the traditional corpus dominating the pool
# by sheer count. Split per family, since motifs are chosen family-first.
DEFAULT_MOTIFS = 600

# Flat probability tables that blend by weighted average.
FLAT = ("intervals", "iois", "rests", "steps")


def _blend_flat(tables, weights):
    """Weighted average of several normalised {key: probability} tables."""
    merged = defaultdict(float)
    for name, table in tables.items():
        weight = weights[name]
        total = sum(float(v) for v in table.values()) or 1.
        for key, value in table.items():
            merged[key] += weight * float(value) / total
    total = sum(merged.values()) or 1.
    return {str(k): round(v / total, 6) for k, v in sorted(merged.items(), key=lambda kv: float(kv[0]))}


def _blend_nested(tables, weights):
    """Weighted average of {context: {key: probability}} tables.

    A context present in only one songbook keeps that songbook's distribution
    rather than being diluted, so a chiptune-only interval pair survives.
    """
    contexts = {key for table in tables.values() for key in table}
    merged = {}
    for context in contexts:
        present = {name: table[context] for name, table in tables.items() if context in table}
        share = sum(weights[name] for name in present) or 1.
        merged[context] = _blend_flat(present, {name: weights[name] / share for name in present})
    return dict(sorted(merged.items(), key=lambda kv: kv[0]))


def _blend_roots(tables, weights):
    """Root-motion tables are keyed by family, then by chord root."""
    families = {family for table in tables.values() for family in table}
    merged = {}
    for family in families:
        present = {name: table[family] for name, table in tables.items() if family in table}
        share = sum(weights[name] for name in present) or 1.
        merged[family] = _blend_nested(present, {name: weights[name] / share for name in present})
    return dict(sorted(merged.items()))


def _blend_motifs(motifs, weights, total, seed):
    """Sample each corpus's motifs in proportion to its weight, per family.

    Motifs are chosen family-first at performance time, so the balance has to
    hold inside each family and not merely overall.
    """
    rng = random.Random(seed)
    families = {motif.get("family") for pool in motifs.values() for motif in pool}
    everything = sum(len(pool) for pool in motifs.values()) or 1
    chosen = []
    for family in sorted(families, key=str):
        pools = {name: [m for m in pool if m.get("family") == family] for name, pool in motifs.items()}
        pools = {name: pool for name, pool in pools.items() if pool}
        if not pools:
            continue
        # Families keep their natural proportion of the budget; inside a family
        # the corpora split it by weight.
        available = sum(len(pool) for pool in pools.values())
        family_total = max(1, round(total * available / everything))
        share = sum(weights[name] for name in pools) or 1.
        # A corpus cannot supply more motifs than it has, and the first attempt
        # at this quietly let the largest corpus absorb every shortfall: asking
        # for 40% chiptune returned 18% because only 64 chiptune motifs exist.
        # The blend is the point, so the pool shrinks to the size the ratio can
        # actually fill rather than the ratio bending to a requested size.
        family_total = min(family_total,
                           *(int(len(pool) * share / weights[name]) for name, pool in pools.items()))
        family_total = max(len(pools), family_total)
        for name, pool in pools.items():
            want = max(1, round(family_total * weights[name] / share))
            picked = pool if want >= len(pool) else rng.sample(pool, want)
            for motif in picked:
                entry = dict(motif)
                entry["corpus"] = name
                chosen.append(entry)
    rng.shuffle(chosen)
    return chosen


def build(weights=None, motif_total=DEFAULT_MOTIFS, seed=20260917, log=print):
    weights = dict(weights or DEFAULT_WEIGHTS)
    share = sum(weights.values())
    if share <= 0:
        raise ValueError("Blend weights must add up to something positive")
    weights = {name: value / share for name, value in weights.items()}

    books = {}
    for name in weights:
        path = OUT / f"{name}.json"
        raw = path.read_bytes()
        manifest = json.loads((OUT / f"{name}.manifest.json").read_text(encoding="utf-8"))
        if hashlib.sha256(raw).hexdigest() != manifest["sha256"]:
            raise ValueError(f"{name}.json does not match its manifest")
        books[name] = json.loads(raw)
        log(f"{name:9s} weight {weights[name]:.2f}  motifs {len(books[name]['motifs']):4d}")

    blended = {"version": max(book.get("version", 1) for book in books.values())}
    for table in FLAT:
        blended[table] = _blend_flat({n: b[table] for n, b in books.items()}, weights)
    blended["interval_bigrams"] = _blend_nested({n: b["interval_bigrams"] for n, b in books.items()}, weights)
    blended["root_transitions"] = _blend_roots({n: b["root_transitions"] for n, b in books.items()}, weights)
    blended["motifs"] = _blend_motifs({n: b["motifs"] for n, b in books.items()}, weights, motif_total, seed)

    # Naturalness is measured against these reference log-probabilities, so they
    # are blended on the same weights rather than taken from one corpus.
    blended["reference"] = {
        key: round(sum(weights[n] * float(b["reference"][key]) for n, b in books.items()), 4)
        for key in ("interval_logprob", "ioi_logprob", "root_logprob")
    }
    blended["reference"]["scope"] = "Weighted blend of each songbook's leave-one-out reference"

    # The playing idiom — drum grids, bass habits, arpeggios, tempo — comes from
    # the retro corpora only. Folk and chorale material has no console idiom and
    # blending one in would invent a tradition that does not exist.
    idiom_source = "console" if "console" in books and "idioms" in books["console"] else next(
        (n for n, b in books.items() if "idioms" in b), None)
    if idiom_source:
        blended["idioms"] = books[idiom_source]["idioms"]
        blended["style"] = books[idiom_source].get("style", idiom_source)

    blended["source"] = {
        "blend": {name: round(weight, 4) for name, weight in weights.items()},
        "corpora": {name: book.get("source", {}) for name, book in books.items()},
        "motif_total": len(blended["motifs"]),
        "idioms_from": idiom_source,
        "note": "Tables are weighted averages of already-normalised distributions, so the "
                "blend is set by weight and not by corpus size.",
    }
    log(f"blended   motifs {len(blended['motifs'])}  idioms from {idiom_source}")
    return blended


def save(data, directory=OUT, name="blended"):
    directory = Path(directory)
    raw = json.dumps(data, separators=(",", ":"), sort_keys=True).encode("utf-8")
    (directory / f"{name}.json").write_bytes(raw)
    manifest = {"sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw),
                "source": data["source"], "reference": data["reference"]}
    (directory / f"{name}.manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--console", type=float, default=DEFAULT_WEIGHTS["console"], help="chiptune share")
    parser.add_argument("--amiga", type=float, default=DEFAULT_WEIGHTS["amiga"], help="tracker module share")
    parser.add_argument("--songbook", type=float, default=DEFAULT_WEIGHTS["songbook"], help="folk and chorale share")
    parser.add_argument("--motifs", type=int, default=DEFAULT_MOTIFS)
    parser.add_argument("--seed", type=int, default=20260917)
    parser.add_argument("--name", default="blended")
    args = parser.parse_args()
    data = build({"console": args.console, "amiga": args.amiga, "songbook": args.songbook},
                 motif_total=args.motifs, seed=args.seed)
    manifest = save(data, name=args.name)
    print(json.dumps({"saved": f"data/foundation/{args.name}.json", "sha256": manifest["sha256"][:16],
                      "blend": data["source"]["blend"], "motifs": len(data["motifs"])}, indent=2), flush=True)


if __name__ == "__main__":
    main()

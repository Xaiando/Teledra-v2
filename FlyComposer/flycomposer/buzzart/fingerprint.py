"""What a tune sounds like from across the room, and how far apart two tunes are.

The critic's ``diversity`` compares notes: which pitches, on which beats. By that
measure three renders of one brain in three styles were 0.69 to 1.00 apart —
very different — and to a listener they were the same tune. Notes are not what
the ear uses to tell music apart at a distance. It hears the groove, which part
is out in front, how long notes ring, how fast the chords move, how fast the
whole thing goes, and what it is played on.

This fingerprint measures those, on fixed scales so that a distance means the
same thing in every batch. A pair closer than :data:`SAME_SOUNDING` is heard as
the same kind of tune whatever its notes are.
"""
from __future__ import annotations

import math
from collections import Counter

import numpy as np

PARTS = ("lead", "answer", "harmony", "bass", "texture")
# The renderer normalises each note, so a part's audible weight is its level
# times its velocity times how much of the piece it fills.
DEFAULT_LEVEL = {"lead": 1., "answer": .66, "harmony": .19, "bass": .72, "texture": .30}
STEPS = 16
# Twelve positions per beat resolve sixteenths (every 3), eighths (every 6),
# triplets (every 4) and swung off-beats (7 to 8). A sixteen-step grid read a
# triplet melody as sixteenths and could not tell them apart.
BEAT_POSITIONS = 12

# Timbre families, so a warm pad and a choir are near each other and a pulse
# wave is far from both.
FAMILIES = {
    "chip": ("chip:pulse12", "chip:pulse25", "chip:pulse50", "chip:triangle", "amiga:lead", "amiga:pluck"),
    "fm": ("fm:lead", "fm:bass", "fm:bell"),
    "drums": ("chip:noise", "amiga:drums", "kick", "snare", "hat", "perc", "shaker", "conga", "cowbell", "timpani"),
    "keys": ("piano", "electric_piano", "honky_tonk_piano", "harpsichord", "clavinet", "keys", "organ", "accordion"),
    "pad": ("pad", "warm_pad", "choir", "synth_brass", "string_ensemble", "tremolo_strings", "amiga:chord"),
    "mallet": ("vibraphone", "marimba", "xylophone", "glockenspiel", "celesta", "tubular_bells", "bell", "dulcimer"),
    "pluck": ("harp", "pluck", "guitar", "electric_guitar", "jazz_guitar", "muted_guitar", "steel_guitar", "banjo",
              "mandolin", "ukulele", "koto", "sitar", "oud", "pizzicato_strings"),
    "bow": ("violin", "viola", "cello"),
    "wind": ("flute", "piccolo", "recorder", "pan_flute", "shakuhachi", "clarinet", "oboe", "english_horn", "bassoon",
             "soprano_sax", "tenor_sax", "harmonica", "reed", "theremin"),
    "brass": ("horn", "trumpet", "trombone", "tuba", "flugelhorn", "brass_ensemble"),
    "bass": ("bass", "sub_bass", "double_bass", "amiga:bass", "classic:bass"),
}
FAMILY_OF = {voice: family for family, voices in FAMILIES.items() for voice in voices}
FAMILY_NAMES = tuple(FAMILIES)

# Below this, two tunes are heard as the same kind of tune: 1.35 times the
# widest pair the operator judged "way too similar" (three renders measuring
# 1.17, 1.77 and 2.00 on this scale). It rests on that one negative judgement,
# so it says only what the operator rejected; which of the pairs above it they
# hear as different is still to be confirmed by ear.
SAME_SOUNDING = 2.7


def family(voice):
    if voice is None:
        return None
    if voice.startswith("classic:"):
        return "keys" if voice != "classic:bass" else "bass"
    return FAMILY_OF.get(voice, "keys")


def fingerprint(score, levels=None):
    """A dict of listener-scale features for one performance."""
    levels = {**DEFAULT_LEVEL, **(score.get("mix") or {}), **(levels or {})}
    events = score["events"]
    beats_per_bar = score.get("beats_per_bar", 4)
    duration = max(1e-6, float(score["duration_seconds"]))
    beat_seconds = 60. / score["bpm"]
    total_beats = duration / beat_seconds
    features = {"tempo": score["bpm"] / 60.}

    by_part = {part: [e for e in events if e["layer"] == part] for part in PARTS}
    weights = {}
    for part in PARTS:
        notes = by_part[part]
        # Where in the bar this part lands, and where inside the beat: the groove.
        # Both are shapes — shares of the part's notes — so a rhythm pattern is
        # compared as a pattern, and how many notes there are stays a separate
        # question for density. As raw counts, a pattern change read as a small
        # density change and all but vanished.
        grid = np.zeros(STEPS)
        within = np.zeros(BEAT_POSITIONS)
        for e in notes:
            step = int(round((e["start_beat"] % beats_per_bar) / beats_per_bar * STEPS)) % STEPS
            grid[step] += 1
            within[int(round((e["start_beat"] % 1.) * BEAT_POSITIONS)) % BEAT_POSITIONS] += 1
        features[f"groove_{part}"] = grid / grid.sum() if grid.sum() else grid
        features[f"beat_{part}"] = within / within.sum() if within.sum() else within
        features[f"density_{part}"] = len(notes) / duration
        lengths = [e["length_beats"] for e in notes]
        features[f"sustain_{part}"] = math.log2(1. + float(np.median(lengths))) if lengths else 0.
        covered = sum(e["duration_seconds"] for e in notes) / duration
        velocity = float(np.mean([e["velocity"] for e in notes])) if notes else 0.
        weights[part] = levels.get(part, 0.) * velocity * min(3., covered) ** .5
        pitched = [e["midi"] for e in notes]
        features[f"register_{part}"] = (float(np.mean(pitched)) - 60.) / 12. if pitched and part != "texture" else 0.

    # Who is out in front, as shares of the total audible weight.
    total = sum(weights.values()) or 1.
    features["mix"] = np.array([weights[p] / total for p in PARTS])

    # Swing: how late the off-beat eighths sit, as a share of an eighth. Only the
    # accompaniment is read. Melody notes landing on the "and-a" of the beat read
    # as swing 0.14 to 0.22 in pieces that had none, which blurred the one axis
    # that most separates a lofi groove from a march.
    offsets = []
    for e in events:
        if e["layer"] not in ("harmony", "bass", "texture"):
            continue
        fraction = e["start_beat"] % 1.
        if .47 <= fraction <= .71:
            offsets.append(fraction - .5)
    features["swing"] = float(np.mean(offsets)) * 2. if len(offsets) >= 8 else 0.

    # How fast the chords move, in changes per bar.
    roots, previous = 0, None
    for e in sorted((e for e in events if e["layer"] == "harmony"), key=lambda e: e["start_beat"]):
        if e["harmonic_root"] != previous:
            roots, previous = roots + 1, e["harmonic_root"]
    features["chord_rate"] = roots / max(1., total_beats / beats_per_bar)

    features["mode"] = np.array([score.get("mode") == m for m in ("minor", "dorian", "major")], dtype=float)
    timbre = np.zeros(len(FAMILY_NAMES))
    for part, voice in (score.get("voices") or {}).items():
        name = family(voice)
        if name in FAMILY_NAMES and by_part.get(part):
            timbre[FAMILY_NAMES.index(name)] += weights.get(part, 0.) / total
    features["timbre"] = timbre
    return features


# How much each aspect counts, and the scale that makes one unit of it audible.
#
# Weighted from what the operator's ear did, not from what seemed likely. The
# pair judged "way too similar" (dreamy against overworld) differed mostly in
# tempo, instruments and whether there were drums — and was still the same tune
# to the listener. What that pair shared was the mix, how often the chords
# moved, a straight feel, and how long notes rang. So those carry the weight
# here, and tempo, timbre and drum density count for less. This is calibrated
# from one negative judgement only; a listener's verdict on tunes that differ in
# the heavily weighted axes is what would confirm it.
_ASPECTS = (
    ("tempo", .35, 25. / 60.),
    ("swing", 2.5, .30),
    ("chord_rate", 2.0, 1.2),
    ("mix", 3.0, .35),
    ("mode", .6, 1.4),
    ("timbre", .7, .8),
)
# Rhythm and length of each part: the pitched parts' patterns are grammar, the
# drums' density was shown not to be enough on its own.
_PART_WEIGHTS = {
    "lead":    {"groove": .6, "density": .5, "sustain": .8, "register": .3},
    "answer":  {"groove": .6, "density": .5, "sustain": .8, "register": .3},
    "harmony": {"groove": .9, "density": .5, "sustain": .8, "register": .3},
    "bass":    {"groove": .9, "density": .5, "sustain": .8, "register": .3},
    "texture": {"groove": .25, "density": .15, "sustain": .3, "register": 0.},
}


def distance(a, b):
    """How different two fingerprints sound; below SAME_SOUNDING is the same kind of tune."""
    total = 0.
    for key, weight, scale in _ASPECTS:
        delta = np.linalg.norm(np.atleast_1d(a[key]) - np.atleast_1d(b[key])) / scale
        total += weight * delta ** 2
    for part in PARTS:
        w = _PART_WEIGHTS[part]
        # Total variation between rhythm shapes, 0 to 1; a third of the notes
        # moving to different places is about one audible unit.
        groove = (.5 * np.abs(a[f"groove_{part}"] - b[f"groove_{part}"]).sum()
                  + .5 * np.abs(a[f"beat_{part}"] - b[f"beat_{part}"]).sum()) / .33
        density = abs(math.log1p(a[f"density_{part}"]) - math.log1p(b[f"density_{part}"])) / .6
        sustain = abs(a[f"sustain_{part}"] - b[f"sustain_{part}"]) / .8
        register = abs(a[f"register_{part}"] - b[f"register_{part}"]) / 1.
        total += (w["groove"] * groove ** 2 + w["density"] * density ** 2
                  + w["sustain"] * sustain ** 2 + w["register"] * register ** 2)
    return math.sqrt(total / 10.)


def spread(scores):
    """Summary of how varied a batch sounds: nearest-neighbour distances and same-sounding pairs."""
    prints = [fingerprint(s) for s in scores]
    n = len(prints)
    pairs, nearest = [], []
    for i in range(n):
        row = [distance(prints[i], prints[j]) for j in range(n) if j != i]
        nearest.append(min(row) if row else 0.)
        pairs += [distance(prints[i], prints[j]) for j in range(i + 1, n)]
    return {"tunes": n, "median_pair": float(np.median(pairs)) if pairs else 0.,
            "median_nearest": float(np.median(nearest)) if nearest else 0.,
            "min_nearest": float(min(nearest)) if nearest else 0.,
            "same_sounding_share": float(np.mean([d < SAME_SOUNDING for d in pairs])) if pairs else 0.}

"""Build Buzzart's songbook from music21's public-domain corpus.

Run once, from the FlyComposer folder, with an environment that has music21
(the Teledra development environment):

  ..\\.venv\\Scripts\\python.exe -m flycomposer.buzzart.songbook_build

Writes data/foundation/songbook.json and songbook.manifest.json. Performing and
training read only those files; they never need music21.

Melodies come from folk collections (Ryan's Mammoth Collection, O'Neill's Music
of Ireland, the Essen folksong collection) and Bach chorale sopranos. Chord-root
motion comes from the Bach chorales. Every motif keeps its collection and title.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import datetime as dt
import hashlib
import json
from pathlib import Path
import random
import time

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "data" / "foundation"
STEP_BINS = (.25, .5, .75, 1., 1.5, 2., 3., 4.)
REST_BINS = (0., .25, .5, 1., 2.)
IOI_BINS = (.25, .5, .75, 1., 1.25, 1.5, 2., 3., 4.)
FOLK = ("ryansMammoth", "oneills1850", "essenFolksong")


def _quantize(value, bins):
    return min(bins, key=lambda b: abs(b - value))


def _melody(part, key):
    """Notes as (degree from tonic, quarter length, rest before, measure, beat)."""
    from music21 import pitch
    tonic = pitch.Pitch(key.tonic.name + "4").diatonicNoteNum
    notes, rest = [], 0.
    for element in part.stripTies(inPlace=False).flatten().notesAndRests:
        length = float(element.quarterLength)
        if length <= 0:
            continue
        if element.isRest:
            rest += length
            continue
        top = element if element.isNote else max(element.notes, key=lambda n: n.pitch.midi)
        notes.append((top.pitch.diatonicNoteNum - tonic, length, rest, element.measureNumber or 0, float(element.beat)))
        rest = 0.
    return notes


def _tokens(notes):
    """Intervals within phrases (a rest of two beats or more starts a new phrase), plus lengths, rests and IOIs."""
    phrases, current = [], []
    for previous, note in zip(notes, notes[1:]):
        if note[2] >= 2.:
            if current:
                phrases.append(current)
            current = []
            continue
        current.append(max(-7, min(7, note[0] - previous[0])))
    if current:
        phrases.append(current)
    steps = [_quantize(n[1], STEP_BINS) for n in notes]
    rests = [_quantize(n[2], REST_BINS) for n in notes[1:]]
    iois = [_quantize(a[1] + b[2], IOI_BINS) for a, b in zip(notes, notes[1:]) if b[2] < 2.]
    return phrases, steps, rests, iois


def _motifs(notes, family, ident, title):
    cells = []
    for i, note in enumerate(notes):
        opening = i == 0 or (note[4] == 1. and note[3] % 4 == 1 and notes[i - 1][3] != note[3])
        segment = notes[i:i + 5]
        if not opening or len(segment) < 5 or any(n[2] > .5 for n in segment[1:]):
            continue
        shift = 7 * (segment[0][0] // 7)
        degrees = [n[0] - shift for n in segment]
        if len(set(degrees)) < 3 or max(degrees) - min(degrees) > 9:
            continue
        steps = [round(min(4., max(.25, a[1] + b[2])), 3) for a, b in zip(segment, segment[1:])] + [round(min(4., segment[-1][1]), 3)]
        cells.append({"id": f"{ident}@bar{note[3]}", "title": title, "family": family, "degrees": degrees, "steps": steps})
    return cells


def _probabilities(counter):
    total = sum(counter.values())
    return {str(k): round(v / total, 6) for k, v in sorted(counter.items())}


def build(max_folk=2400, max_chorales=240, holdout=.1, seed=20260915, log=print):
    from music21 import corpus, pitch
    started = time.perf_counter()
    rng = random.Random(seed)
    tunes, sources = [], Counter()
    paths = [p for collection in FOLK for p in corpus.getComposer(collection)]
    for path_index, path in enumerate(paths):
        if len(tunes) >= max_folk:
            break
        collection = next(c for c in FOLK if c in str(path))
        try:
            parsed = corpus.parse(path)
        except Exception as error:
            log(f"skip {path}: {type(error).__name__}")
            continue
        scores = list(parsed.scores) if hasattr(parsed, "scores") else [parsed]
        for number, score in enumerate(scores):
            if len(tunes) >= max_folk:
                break
            try:
                key = score.analyze("key")
                notes = _melody(score.parts[0] if score.parts else score, key)
            except Exception:
                continue
            if len(notes) < 16:
                continue
            title = (score.metadata.title if score.metadata and score.metadata.title else "untitled")[:80]
            tunes.append({"id": f"{collection}:{Path(str(path)).stem}#{number}", "title": title,
                          "family": "major" if key.mode == "major" else "minor", "notes": notes})
            sources[collection] += 1
        if path_index % 20 == 0:
            log(f"folk: {len(tunes)} melodies after {path_index + 1} files ({time.perf_counter() - started:.0f}s)")
    transitions = {"major": defaultdict(Counter), "minor": defaultdict(Counter)}
    chorale_count = 0
    for chorale in corpus.chorales.Iterator(returnType="stream"):
        if chorale_count >= max_chorales:
            break
        try:
            key = chorale.analyze("key")
            family = "major" if key.mode == "major" else "minor"
            notes = _melody(chorale.parts[0], key)
            tonic = pitch.Pitch(key.tonic.name + "4").diatonicNoteNum
            roots = []
            for chord in chorale.chordify().recurse().getElementsByClass("Chord"):
                if chord.beatStrength < .5 or len(chord.pitches) < 3:
                    continue
                degree = (pitch.Pitch(chord.root().name + "4").diatonicNoteNum - tonic) % 7
                if not roots or roots[-1] != degree:
                    roots.append(degree)
        except Exception:
            continue
        for a, b in zip(roots, roots[1:]):
            transitions[family][a][b] += 1
        title = (chorale.metadata.title if chorale.metadata and chorale.metadata.title else "chorale")[:80]
        if len(notes) >= 16:
            tunes.append({"id": f"bach:chorale#{chorale_count}", "title": title, "family": family, "notes": notes})
            sources["bach chorale sopranos"] += 1
        chorale_count += 1
        if chorale_count % 40 == 0:
            log(f"chorales: {chorale_count} ({time.perf_counter() - started:.0f}s)")

    rng.shuffle(tunes)
    held = tunes[:max(1, int(len(tunes) * holdout))]
    train = tunes[len(held):]

    def tables(selection):
        intervals, bigrams, steps, rests, iois = Counter(), defaultdict(Counter), Counter(), Counter(), Counter()
        for tune in selection:
            phrases, tune_steps, tune_rests, tune_iois = _tokens(tune["notes"])
            for phrase in phrases:
                intervals.update(phrase)
                for a, b in zip(phrase, phrase[1:]):
                    bigrams[a][b] += 1
            steps.update(tune_steps)
            rests.update(tune_rests)
            iois.update(tune_iois)
        return {"intervals": _probabilities(intervals),
                "interval_bigrams": {str(a): {str(b): c for b, c in sorted(row.items())} for a, row in sorted(bigrams.items())},
                "steps": _probabilities(steps), "rests": _probabilities(rests), "iois": _probabilities(iois)}

    root_table = {family: {str(a): _probabilities(row) for a, row in sorted(rows.items())} for family, rows in transitions.items()}
    motifs, seen = [], set()
    for tune in train:
        for cell in _motifs(tune["notes"], tune["family"], tune["id"], tune["title"]):
            signature = (tuple(cell["degrees"]), tuple(cell["steps"]))
            if signature not in seen:
                seen.add(signature)
                motifs.append(cell)
    rng.shuffle(motifs)
    motifs = motifs[:900]

    from .foundation import Foundation
    provisional = {**tables(train), "root_transitions": root_table, "motifs": motifs or [{"id": "none", "family": "major", "degrees": [0, 2, 4], "steps": [1, 1, 1]}],
                   "reference": {"interval_logprob": 0., "ioi_logprob": 0., "root_logprob": 0.}}
    model = Foundation(provisional)
    interval_values, ioi_values = [], []
    for tune in held:
        phrases, _, _, tune_iois = _tokens(tune["notes"])
        for phrase in phrases:
            interval_values += model.interval_logprobs(phrase)
        ioi_values += model.ioi_logprobs(tune_iois)
    root_values = [value for family, rows in root_table.items() for a, row in rows.items() for b, p in row.items()
                   for value in [model.root_logprobs([int(a), int(b)], family)[0]] * max(1, int(round(p * 100)))]
    mean = lambda values: sum(values) / len(values) if values else 0.
    data = {"version": 1, **tables(tunes), "root_transitions": root_table, "motifs": motifs,
            "reference": {"interval_logprob": round(mean(interval_values), 4), "ioi_logprob": round(mean(ioi_values), 4),
                          "root_logprob": round(mean(root_values), 4),
                          "scope": f"Mean log-probability per token on {len(held)} held-out melodies under tables from the other {len(train)}"},
            "source": {"library": "music21 10.5.0 corpus", "corpora": dict(sources), "chorales_for_harmony": chorale_count,
                       "works": len(tunes), "melodies": len(tunes), "notes": sum(len(t["notes"]) for t in tunes),
                       "licensing": "Public-domain folk collections and Bach chorales as bundled with music21 (BSD-3-Clause library).",
                       "built_at": dt.datetime.now(dt.timezone.utc).isoformat(), "build_seconds": round(time.perf_counter() - started, 1)}}
    return data


def save(data, directory=OUT):
    directory.mkdir(parents=True, exist_ok=True)
    raw = json.dumps(data, separators=(",", ":"), sort_keys=True).encode("utf-8")
    path = directory / "songbook.json"
    pending = path.with_name("songbook.next.json")
    pending.write_bytes(raw)
    pending.replace(path)
    manifest = {"file": path.name, "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw),
                "source": data["source"], "reference": data["reference"]}
    (directory / "songbook.manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--max-folk", type=int, default=2400)
    parser.add_argument("--max-chorales", type=int, default=240)
    args = parser.parse_args()
    data = build(args.max_folk, args.max_chorales, log=lambda line: print(line, flush=True))
    manifest = save(data)
    print(json.dumps({"saved": str(OUT / "songbook.json"), "sha256": manifest["sha256"][:16], "bytes": manifest["bytes"],
                      "source": data["source"], "reference": data["reference"], "motifs": len(data["motifs"])}, indent=2), flush=True)


if __name__ == "__main__":
    main()

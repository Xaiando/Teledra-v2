"""Build Buzzart's console songbook from openly licensed chiptune MIDI files.

Run from the FlyComposer folder with an environment that has music21:

  ..\\.venv\\Scripts\\python.exe -m flycomposer.buzzart.console_build

Sources are the CC0 files recorded in data/foundation/sources/opengameart/sources.json.
Parts are sorted into roles (lead, bass, arpeggio, pad, drums) by register, speed
and note length, because these files carry no instrument names. The lead line
gives tables in the same shape as the classical songbook, so one Foundation class
reads either. Console idioms are added on top: tempo, bass pulse and octave
jumps, arpeggio speed and shape, and 16-step kick/snare/hat grids.
"""
from __future__ import annotations

from collections import Counter, defaultdict
import datetime as dt
import hashlib
import io
import json
from pathlib import Path
import zipfile

from .songbook_build import IOI_BINS, _motifs, _probabilities, _quantize, _tokens

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ROOT / "data" / "foundation" / "sources" / "opengameart"
OUT = ROOT / "data" / "foundation"
KICK, SNARE, HAT = {35, 36}, {37, 38, 39, 40}, {42, 44, 46, 51, 53, 59}


def _pieces():
    records = json.loads((SOURCES / "sources.json").read_text(encoding="utf-8"))
    for record in records:
        path = SOURCES / record["file"]
        if path.suffix == ".zip":
            with zipfile.ZipFile(path) as archive:
                for member in archive.namelist():
                    if member.lower().endswith(".mid"):
                        yield f"{record['title']} / {member}", archive.read(member), record
        else:
            yield record["title"], path.read_bytes(), record


def _drum_hits(part):
    """(offset, category) for each hit; percussion chords contribute every stroke."""
    hits = []
    for element in part.flatten().notes:
        strokes = list(element.notes) if element.isChord else [element]
        for stroke in strokes:
            stored = getattr(stroke, "storedInstrument", None)
            number = getattr(stored, "percMapPitch", None) if stored is not None else None
            category = "kick" if number in KICK else "snare" if number in SNARE else "hat" if number in HAT else None
            if category:
                hits.append((float(element.offset), category))
    return hits


def _profile(part):
    notes = [n for n in part.flatten().notes if getattr(n, "pitches", ())]
    if len(notes) < 12:
        return None
    pitches = [max(p.midi for p in n.pitches) for n in notes]
    onsets = sorted({float(n.offset) for n in notes})
    iois = sorted(b - a for a, b in zip(onsets, onsets[1:]) if b > a)
    lengths = sorted(float(n.quarterLength) for n in notes)
    signature = hashlib.sha256(json.dumps([(float(n.offset), max(p.midi for p in n.pitches)) for n in notes]).encode()).hexdigest()
    line = [max(p.midi for p in n.pitches) for n in sorted(notes, key=lambda n: float(n.offset))]
    return {"part": part, "notes": notes, "mean": sum(pitches) / len(pitches), "low": min(pitches), "high": max(pitches),
            "ioi": iois[len(iois) // 2] if iois else 4., "length": lengths[len(lengths) // 2], "signature": signature,
            "chord_share": sum(1 for n in notes if n.isChord) / len(notes),
            "leap_share": sum(abs(b - a) >= 5 for a, b in zip(line, line[1:])) / max(1, len(line) - 1)}


def _roles(score):
    profiles, seen = [], set()
    drums = None
    for part in score.parts:
        if any(not getattr(n, "pitches", ()) for n in part.flatten().notes):
            drums = drums or part
            continue
        profile = _profile(part)
        if profile and profile["signature"] not in seen:  # doubled tracks count once
            seen.add(profile["signature"])
            profiles.append(profile)
    if not profiles:
        return None
    bass = min(profiles, key=lambda p: p["mean"])
    # A part that leaps on most of its notes is a broken chord, not a tune, however high it sits.
    tunes = [p for p in profiles if p is not bass and p["leap_share"] <= .5]
    melodic = [p for p in tunes if p["length"] <= 1.5 and p["chord_share"] < .3]
    lead = max(melodic or tunes, key=lambda p: (p["mean"], -p["ioi"]), default=None)
    arpeggios = [p for p in profiles if p not in (bass, lead) and p["ioi"] <= .34 and p["high"] - p["low"] >= 10]
    pads = [p for p in profiles if p not in (bass, lead) and p["length"] >= 2.]
    return {"lead": lead, "bass": bass, "arpeggios": arpeggios, "pads": pads, "drums": drums}


def build(log=print):
    import warnings
    warnings.filterwarnings("ignore")
    from music21 import converter, meter, pitch, tempo
    pieces, tempos, credits = [], [], {}
    bass_iois, bass_intervals, arpeggio_rates, arpeggio_shapes, arpeggio_spans = Counter(), Counter(), Counter(), Counter(), []
    step_hits = {name: [0] * 16 for name in ("kick", "snare", "hat")}
    patterns, drum_bars = Counter(), 0
    transitions = {"major": defaultdict(Counter), "minor": defaultdict(Counter)}
    for label, data, record in _pieces():
        try:
            score = converter.parseData(data, format="midi")
            key = score.analyze("key")
        except Exception as error:
            log(f"skip {label}: {type(error).__name__}")
            continue
        credits[record["title"]] = {"author": record["author"], "license": record["license"], "page": record["page"]}
        marks = [m.number for m in score.recurse().getElementsByClass(tempo.MetronomeMark) if m.number]
        if marks:
            tempos.append(round(marks[0]))
        four_four = all(ts.ratioString == "4/4" for ts in score.recurse().getElementsByClass(meter.TimeSignature))
        roles = _roles(score)
        if not roles:
            log(f"skip {label}: no pitched parts")
            continue
        family = "major" if key.mode == "major" else "minor"
        from .songbook_build import _melody
        # A piece without a tune still teaches bass, arpeggio and drum idioms.
        lead_notes = _melody(roles["lead"]["part"], key) if roles["lead"] else []
        if lead_notes:
            pieces.append({"id": f"console:{label}", "title": label, "family": family, "notes": lead_notes})
        tonic = pitch.Pitch(key.tonic.name + "4").diatonicNoteNum
        bass_notes = sorted(roles["bass"]["notes"], key=lambda n: float(n.offset))
        onsets = [float(n.offset) for n in bass_notes]
        bass_iois.update(_quantize(b - a, IOI_BINS) for a, b in zip(onsets, onsets[1:]) if 0 < b - a <= 4)
        midis = [min(p.midi for p in n.pitches) for n in bass_notes]
        bass_intervals.update(max(-12, min(12, b - a)) for a, b in zip(midis, midis[1:]))
        roots = []
        for n in bass_notes:
            if abs(float(n.offset) % 2.) < 1e-6:
                degree = (pitch.Pitch(min(n.pitches, key=lambda p: p.midi).name + "4").diatonicNoteNum - tonic) % 7
                if not roots or roots[-1] != degree:
                    roots.append(degree)
        for a, b in zip(roots, roots[1:]):
            transitions[family][a][b] += 1
        for profile in roles["arpeggios"]:
            arpeggio_rates[round(1. / profile["ioi"])] += 1
            arpeggio_spans.append(profile["high"] - profile["low"])
            line = [max(p.midi for p in n.pitches) for n in sorted(profile["notes"], key=lambda n: float(n.offset))]
            for i in range(0, len(line) - 3, 4):
                a, b, c, d = line[i:i + 4]
                arpeggio_shapes["up" if a < b < c else "down" if a > b > c else "updown" if a < b > c or a > b < c else "other"] += 1
        if roles["drums"] is not None and four_four:
            bars = defaultdict(lambda: {"kick": ["0"] * 16, "snare": ["0"] * 16, "hat": ["0"] * 16})
            for offset, category in _drum_hits(roles["drums"]):
                step = int(round((offset % 4.) * 4.)) % 16
                bars[int(offset // 4.)][category][step] = "1"
            for bar in bars.values():
                drum_bars += 1
                for category in step_hits:
                    for step, hit in enumerate(bar[category]):
                        step_hits[category][step] += hit == "1"
                patterns[(tuple(bar["kick"]), tuple(bar["snare"]), tuple(bar["hat"]))] += 1
        log(f"{label}: lead {len(lead_notes)} notes, bass {len(bass_notes)}, arpeggio parts {len(roles['arpeggios'])}, "
            f"drums {'yes' if roles['drums'] is not None else 'no'}, {key}")

    def tables(selection):
        intervals, bigrams, steps, rests, iois = Counter(), defaultdict(Counter), Counter(), Counter(), Counter()
        from .songbook_build import REST_BINS, STEP_BINS  # noqa: F401  (bins used by _tokens)
        for piece in selection:
            phrases, piece_steps, piece_rests, piece_iois = _tokens(piece["notes"])
            for phrase in phrases:
                intervals.update(phrase)
                for a, b in zip(phrase, phrase[1:]):
                    bigrams[a][b] += 1
            steps.update(piece_steps)
            rests.update(piece_rests)
            iois.update(piece_iois)
        return {"intervals": _probabilities(intervals),
                "interval_bigrams": {str(a): {str(b): c for b, c in sorted(row.items())} for a, row in sorted(bigrams.items())},
                "steps": _probabilities(steps), "rests": _probabilities(rests), "iois": _probabilities(iois)}

    root_table = {family: {str(a): _probabilities(row) for a, row in sorted(rows.items())} for family, rows in transitions.items()}
    motifs, seen = [], set()
    for piece in pieces:
        for cell in _motifs(piece["notes"], piece["family"], piece["id"], piece["title"]):
            signature = (tuple(cell["degrees"]), tuple(cell["steps"]))
            if signature not in seen:
                seen.add(signature)
                motifs.append(cell)
    from .foundation import Foundation
    interval_values, ioi_values = [], []
    for held in pieces:  # leave one piece out so the reference is not in-sample
        others = [p for p in pieces if p is not held]
        model = Foundation({**tables(others), "root_transitions": root_table, "motifs": motifs or [{"id": "x", "family": "major", "degrees": [0, 2, 4], "steps": [1, 1, 1]}],
                            "reference": {"interval_logprob": 0., "ioi_logprob": 0., "root_logprob": 0.}})
        phrases, _, _, piece_iois = _tokens(held["notes"])
        for phrase in phrases:
            interval_values += model.interval_logprobs(phrase)
        ioi_values += model.ioi_logprobs(piece_iois)
    full = Foundation({**tables(pieces), "root_transitions": root_table, "motifs": motifs,
                       "reference": {"interval_logprob": 0., "ioi_logprob": 0., "root_logprob": 0.}})
    root_values = [v for family, rows in transitions.items() for a, row in rows.items() for b, count in row.items()
                   for v in [full.root_logprobs([a, b], family)[0]] * count]
    mean = lambda values: round(sum(values) / len(values), 4) if values else 0.
    bars = max(1, drum_bars)
    common = [{"kick": "".join(k), "snare": "".join(s), "hat": "".join(h), "bars": count}
              for (k, s, h), count in patterns.most_common(32) if "1" in "".join(k) + "".join(s) + "".join(h)]
    return {"version": 1, "style": "console", **tables(pieces), "root_transitions": root_table, "motifs": motifs,
            "reference": {"interval_logprob": mean(interval_values), "ioi_logprob": mean(ioi_values), "root_logprob": mean(root_values),
                          "scope": f"Leave-one-piece-out mean log-probability per token over {len(pieces)} pieces"},
            "idioms": {"tempo_bpm": sorted(tempos),
                       "bass": {"iois": _probabilities(bass_iois), "intervals": _probabilities(bass_intervals),
                                "octave_share": round(sum(c for i, c in bass_intervals.items() if abs(i) == 12) / max(1, sum(bass_intervals.values())), 4),
                                "repeat_share": round(bass_intervals.get(0, 0) / max(1, sum(bass_intervals.values())), 4)},
                       "arpeggio": {"parts": sum(arpeggio_rates.values()), "notes_per_beat": _probabilities(arpeggio_rates) if arpeggio_rates else {},
                                    "shapes": _probabilities(arpeggio_shapes) if arpeggio_shapes else {},
                                    "median_span": sorted(arpeggio_spans)[len(arpeggio_spans) // 2] if arpeggio_spans else None},
                       "drums": {"bars": drum_bars, "step_probability": {k: [round(v / bars, 4) for v in hits] for k, hits in step_hits.items()},
                                 "patterns": common}},
            "source": {"corpora": {"opengameart CC0 MIDI": len(pieces)}, "works": len(pieces), "melodies": len(pieces),
                       "notes": sum(len(p["notes"]) for p in pieces), "credits": credits,
                       "licensing": "Every source is CC0 (public domain dedication) on OpenGameArt; see sources.json for pages and hashes.",
                       "built_at": dt.datetime.now(dt.timezone.utc).isoformat()}}


def save(data, directory=OUT):
    raw = json.dumps(data, separators=(",", ":"), sort_keys=True).encode("utf-8")
    path = directory / "console.json"
    pending = path.with_name("console.next.json")
    pending.write_bytes(raw)
    pending.replace(path)
    manifest = {"file": path.name, "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw),
                "source": data["source"], "reference": data["reference"]}
    (directory / "console.manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


if __name__ == "__main__":
    result = build(log=lambda line: print(line, flush=True))
    manifest = save(result)
    idioms = result["idioms"]
    print(json.dumps({"saved": "data/foundation/console.json", "sha256": manifest["sha256"][:16], "pieces": result["source"]["works"],
                      "motifs": len(result["motifs"]), "reference": result["reference"], "tempo_bpm": idioms["tempo_bpm"],
                      "bass": {k: idioms["bass"][k] for k in ("octave_share", "repeat_share")}, "bass_iois": idioms["bass"]["iois"],
                      "arpeggio": idioms["arpeggio"], "drum_bars": idioms["drums"]["bars"],
                      "top_drum_patterns": idioms["drums"]["patterns"][:5], "lead_intervals": result["intervals"]}, indent=1), flush=True)

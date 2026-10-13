"""Build Buzzart's Amiga songbook from public-domain ProTracker modules.

  ..\\.venv\\Scripts\\python.exe -m flycomposer.buzzart.amiga_build

Reads the modules recorded in data/foundation/sources/modarchive/sources.json,
each listed as Public Domain on The Mod Archive. Module authors name samples
freely, so drums are found by behaviour: a sample played at one or two pitches,
many times, in short notes is percussion, and where it lands in the bar says
whether it acts as a kick (beats one and three), a snare (two and four) or a hat
(between). The lead is the highest busy channel that moves like a melody (several
pitches over a real range, not one note retriggered) and the bass the lowest.
Writes data/foundation/amiga.json in the console songbook's shape, plus how the
classic tracker arpeggio effect is used.
"""
from __future__ import annotations

from collections import Counter, defaultdict
import datetime as dt
import hashlib
import json
import math
from pathlib import Path

from .mod_reader import read_mod
from .songbook_build import IOI_BINS, _motifs, _probabilities, _quantize, _tokens

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ROOT / "data" / "foundation" / "sources" / "modarchive"
OUT = ROOT / "data" / "foundation"
MAJOR_PROFILE = (6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88)
MINOR_PROFILE = (6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17)
SCALE = {"major": (0, 2, 4, 5, 7, 9, 11), "minor": (0, 2, 3, 5, 7, 8, 10)}


def _correlation(a, b):
    mean_a, mean_b = sum(a) / len(a), sum(b) / len(b)
    numerator = sum((x - mean_a) * (y - mean_b) for x, y in zip(a, b))
    denominator = math.sqrt(sum((x - mean_a) ** 2 for x in a) * sum((y - mean_b) ** 2 for y in b))
    return numerator / denominator if denominator else -1.


def estimate_key(notes):
    """Krumhansl-Kessler key from duration-weighted pitch classes: (tonic pitch class, mode)."""
    histogram = [0.] * 12
    for note in notes:
        histogram[note["midi"] % 12] += note["beats"]
    _, tonic, mode = max((_correlation([profile[(pc - tonic) % 12] for pc in range(12)], histogram), tonic, mode)
                         for mode, profile in (("major", MAJOR_PROFILE), ("minor", MINOR_PROFILE)) for tonic in range(12))
    return tonic, mode


def degree(midi, tonic, mode):
    """Scale degree counted from the tonic nearest middle C; chromatic notes snap down to the scale."""
    octave, within = divmod(midi - 60 - tonic, 12)
    return octave * 7 + max(i for i, step in enumerate(SCALE[mode]) if step <= within)


def _tuples(line, tonic, mode):
    tuples, previous_end = [], None
    for note in line:
        rest = max(0., note["beat"] - previous_end) if previous_end is not None else 0.
        tuples.append((degree(note["midi"], tonic, mode), note["beats"], rest, int(note["beat"] // 4) + 1, note["beat"] % 4 + 1.))
        previous_end = note["beat"] + note["beats"]
    return tuples


def _line_shape(line):
    pitches = [n["midi"] for n in line]
    return {"mean": sum(pitches) / len(pitches), "pitches": len(set(pitches)), "range": max(pitches) - min(pitches),
            "repeat_share": sum(a == b for a, b in zip(pitches, pitches[1:])) / max(1, len(pitches) - 1)}


def drum_samples(song):
    """sample number -> kick, snare or hat, judged by pitch variety, count, length and bar position."""
    usage = defaultdict(list)
    for note in song["notes"]:
        usage[note["sample"]].append(note)
    drums = {}
    for sample, hits in usage.items():
        if not sample or len(hits) < 16 or len({h["midi"] for h in hits}) > 2:
            continue
        if sum(h["beats"] for h in hits) / len(hits) > 1.5:
            continue  # a held one-note part is a pad, not a drum
        steps = Counter(int(round((h["beat"] % 4) * 4)) % 16 for h in hits)
        total = sum(steps.values())
        kick, snare = (steps[0] + steps[8]) / total, (steps[4] + steps[12]) / total
        drums[sample] = max((kick * 1.4, "kick"), (snare * 1.4, "snare"), (1. - kick - snare, "hat"))[1]
    return drums


def build(log=print):
    records = json.loads((SOURCES / "sources.json").read_text(encoding="utf-8"))
    pieces, tempos, credits = [], [], {}
    bass_iois, bass_intervals, effect_chords = Counter(), Counter(), Counter()
    arpeggio_rates, arpeggio_spans = Counter(), []
    effect_notes = total_notes = 0
    step_hits = {name: [0] * 16 for name in ("kick", "snare", "hat")}
    patterns, drum_bars = Counter(), 0
    transitions = {"major": defaultdict(Counter), "minor": defaultdict(Counter)}
    for record in records:
        try:
            song = read_mod((SOURCES / record["file"]).read_bytes())
        except Exception as error:
            log(f"skip {record['file']}: {type(error).__name__}: {error}")
            continue
        drums = drum_samples(song)
        melodic = [n for n in song["notes"] if n["sample"] not in drums]
        if len(melodic) < 32:
            log(f"skip {record['file']}: too few melodic notes")
            continue
        tonic, mode = estimate_key(melodic)
        channels = defaultdict(list)
        for note in melodic:
            channels[note["channel"]].append(note)
        for line in channels.values():
            line.sort(key=lambda n: n["beat"])
        # Modules with many channels spread the melody thinly; fall back to the three busiest channels.
        busy = ([c for c, line in channels.items() if len(line) >= .1 * len(melodic)]
                or sorted(channels, key=lambda c: -len(channels[c]))[:3])
        shape = {c: _line_shape(channels[c]) for c in busy}
        # A lead moves: several pitches over a real range, not one note retriggered.
        tuneful = [c for c in busy if shape[c]["pitches"] >= 5 and shape[c]["range"] >= 7 and shape[c]["repeat_share"] <= .6]
        if not tuneful:
            log(f"skip {record['file']}: no channel moves like a melody")
            continue
        lead_channel = max(tuneful, key=lambda c: shape[c]["mean"])
        bass_channel = min((c for c in busy if shape[c]["pitches"] >= 2), key=lambda c: shape[c]["mean"], default=lead_channel)
        credits[record["title"] or record["file"]] = {"module_id": record["id"], "license": record["license"], "page": record["page"]}
        tempos.append(song["bpm"])
        total_notes += len(melodic)
        for note in melodic:
            if note["arpeggio"]:
                effect_notes += 1
                effect_chords[",".join(str(v) for v in note["arpeggio"])] += 1
        lead = sorted(channels[lead_channel], key=lambda n: n["beat"])
        lead_tuples = _tuples(lead, tonic, mode)
        pieces.append({"id": f"amiga:{record['file']}", "title": record["title"] or record["file"], "family": mode, "notes": lead_tuples})
        if bass_channel != lead_channel:
            bass = sorted(channels[bass_channel], key=lambda n: n["beat"])
            bass_iois.update(_quantize(b["beat"] - a["beat"], IOI_BINS) for a, b in zip(bass, bass[1:]) if 0 < b["beat"] - a["beat"] <= 4)
            bass_intervals.update(max(-12, min(12, b["midi"] - a["midi"])) for a, b in zip(bass, bass[1:]))
            roots = []
            for note in bass:
                if abs(note["beat"] % 2.) < 1e-6:
                    value = degree(note["midi"], tonic, mode) % 7
                    if not roots or roots[-1] != value:
                        roots.append(value)
            for a, b in zip(roots, roots[1:]):
                transitions[mode][a][b] += 1
        for channel, line in channels.items():
            if channel in (lead_channel, bass_channel) or len(line) < 32:
                continue
            onsets = sorted(n["beat"] for n in line)
            gaps = sorted(b - a for a, b in zip(onsets, onsets[1:]) if b > a)
            span = max(n["midi"] for n in line) - min(n["midi"] for n in line)
            if gaps and gaps[len(gaps) // 2] <= .34 and span >= 10:
                arpeggio_rates[round(1. / gaps[len(gaps) // 2])] += 1
                arpeggio_spans.append(span)
        if drums:
            bars = defaultdict(lambda: {"kick": ["0"] * 16, "snare": ["0"] * 16, "hat": ["0"] * 16})
            for note in song["notes"]:
                category = drums.get(note["sample"])
                if category:
                    bars[int(note["beat"] // 4)][category][int(round((note["beat"] % 4) * 4)) % 16] = "1"
            for bar in bars.values():
                drum_bars += 1
                for category in step_hits:
                    for step, hit in enumerate(bar[category]):
                        step_hits[category][step] += hit == "1"
                patterns[("".join(bar["kick"]), "".join(bar["snare"]), "".join(bar["hat"]))] += 1
        log(f"{record['file']}: {song['bpm']} BPM, {('C C# D Eb E F F# G Ab A Bb B'.split())[tonic]} {mode}, lead ch{lead_channel} "
            f"{len(lead)} notes, bass ch{bass_channel}, drums {dict(Counter(drums.values()))}")

    def tables(selection):
        intervals, bigrams, steps, rests, iois = Counter(), defaultdict(Counter), Counter(), Counter(), Counter()
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
    placeholder = motifs or [{"id": "x", "family": "major", "degrees": [0, 2, 4], "steps": [1, 1, 1]}]
    interval_values, ioi_values = [], []
    for held in pieces:
        model = Foundation({**tables([p for p in pieces if p is not held]), "root_transitions": root_table, "motifs": placeholder,
                            "reference": {"interval_logprob": 0., "ioi_logprob": 0., "root_logprob": 0.}})
        phrases, _, _, piece_iois = _tokens(held["notes"])
        for phrase in phrases:
            interval_values += model.interval_logprobs(phrase)
        ioi_values += model.ioi_logprobs(piece_iois)
    full = Foundation({**tables(pieces), "root_transitions": root_table, "motifs": placeholder,
                       "reference": {"interval_logprob": 0., "ioi_logprob": 0., "root_logprob": 0.}})
    root_values = [v for family, rows in transitions.items() for a, row in rows.items() for b, count in row.items()
                   for v in [full.root_logprobs([a, b], family)[0]] * count]
    average = lambda values: round(sum(values) / len(values), 4) if values else 0.
    bars = max(1, drum_bars)
    return {"version": 1, "style": "amiga", **tables(pieces), "root_transitions": root_table, "motifs": motifs,
            "reference": {"interval_logprob": average(interval_values), "ioi_logprob": average(ioi_values),
                          "root_logprob": average(root_values),
                          "scope": f"Leave-one-module-out mean log-probability per token over {len(pieces)} modules"},
            "idioms": {"tempo_bpm": sorted(tempos),
                       "bass": {"iois": _probabilities(bass_iois) if bass_iois else {}, "intervals": _probabilities(bass_intervals) if bass_intervals else {},
                                "octave_share": round(sum(c for i, c in bass_intervals.items() if abs(i) == 12) / max(1, sum(bass_intervals.values())), 4),
                                "repeat_share": round(bass_intervals.get(0, 0) / max(1, sum(bass_intervals.values())), 4)},
                       "arpeggio": {"parts": sum(arpeggio_rates.values()),
                                    "notes_per_beat": _probabilities(arpeggio_rates) if arpeggio_rates else {"4": 1.},
                                    "median_span": sorted(arpeggio_spans)[len(arpeggio_spans) // 2] if arpeggio_spans else None},
                       "arpeggio_effect": {"share_of_notes": round(effect_notes / max(1, total_notes), 4),
                                           "chords": _probabilities(effect_chords) if effect_chords else {}},
                       "drums": {"bars": drum_bars, "step_probability": {k: [round(v / bars, 4) for v in hits] for k, hits in step_hits.items()},
                                 "patterns": [{"kick": k, "snare": s, "hat": h, "bars": count}
                                              for (k, s, h), count in patterns.most_common(32) if "1" in k + s + h]}},
            "source": {"corpora": {"The Mod Archive public-domain ProTracker modules": len(pieces)}, "works": len(pieces),
                       "melodies": len(pieces), "notes": sum(len(p["notes"]) for p in pieces), "credits": credits,
                       "licensing": "Every module is listed under the Public Domain license on The Mod Archive; see sources.json for pages and hashes.",
                       "built_at": dt.datetime.now(dt.timezone.utc).isoformat()}}


def save(data, directory=OUT):
    raw = json.dumps(data, separators=(",", ":"), sort_keys=True).encode("utf-8")
    path = directory / "amiga.json"
    pending = path.with_name("amiga.next.json")
    pending.write_bytes(raw)
    pending.replace(path)
    manifest = {"file": path.name, "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw),
                "source": data["source"], "reference": data["reference"]}
    (directory / "amiga.manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


if __name__ == "__main__":
    result = build(log=lambda line: print(line, flush=True))
    manifest = save(result)
    idioms = result["idioms"]
    print(json.dumps({"saved": "data/foundation/amiga.json", "sha256": manifest["sha256"][:16], "modules": result["source"]["works"],
                      "lead_notes": result["source"]["notes"], "motifs": len(result["motifs"]), "reference": result["reference"],
                      "tempo_bpm": idioms["tempo_bpm"], "bass": {k: idioms["bass"][k] for k in ("octave_share", "repeat_share")},
                      "arpeggio": idioms["arpeggio"], "arpeggio_effect": idioms["arpeggio_effect"], "drum_bars": idioms["drums"]["bars"],
                      "step_probability": idioms["drums"]["step_probability"], "intervals": result["intervals"], "iois": result["iois"]}, indent=1), flush=True)

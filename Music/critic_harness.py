"""Measure Teledra PyMusic renders so a critic can judge the music.

The harness replays a render's source inside the worker's sandbox with the
note helpers instrumented, recording when every note sounds, its pitch,
instrument and level. It then analyses the rendered audio.wav, compares the
piece with the accepted render before it, and writes analysis.json,
summary.txt and critic_view.png (piano roll, spectrogram, loudness).

  .venv\\Scripts\\python.exe Music\\critic_harness.py --current
  .venv\\Scripts\\python.exe Music\\critic_harness.py --history 8
  .venv\\Scripts\\python.exe Music\\critic_harness.py RENDER_DIR [RENDER_DIR ...]
  .venv\\Scripts\\python.exe Music\\critic_harness.py --wav path\\to\\current.wav

Metrics are evidence, not verdicts. The harness cannot hear; its observations
are thresholds for a critic to confirm against the plots and the source.
"""
from __future__ import annotations

import argparse
import ast
from collections import Counter
from datetime import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np

HERE = Path(__file__).resolve().parent
HARNESS_VERSION = 3
CAPTURE_TIMEOUT = 180
NOTE_NAMES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"]
# Krumhansl-Kessler key profiles.
MAJOR_PROFILE = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
MINOR_PROFILE = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])
MAJOR_DEGREES = (0, 2, 4, 5, 7, 9, 11)
# Natural minor plus the raised sixth and seventh of melodic/harmonic minor.
MINOR_DEGREES = (0, 2, 3, 5, 7, 8, 9, 10, 11)
NOISE_WAVES = {"pink", "pink_noise", "noise", "white", "white_noise"}
# Synth helpers that transform a whole array without moving its notes in time.
PRESERVING = ("stereo_pan", "stereo_width", "apply_automation", "soft_limiter", "delay",
              "lowpass_filter", "reverb", "granular_synthesis", "fit_to_length",
              "make_seamless_loop")


def data_root() -> Path:
    base = os.environ.get("TELEDRA_DATA_DIR") or os.path.join(
        os.environ.get("LOCALAPPDATA", "."), "Teledra V2")
    return Path(base) / "music"


# --------------------------------------------------------------- capture ----

def capture(source: str) -> dict:
    """Replay `source` with instrumented helpers. Runs in a child process."""
    sys.path.insert(0, str(HERE))
    import inspect
    import python_music_worker as worker
    import teledra_synth as synth

    notes: list[dict] = []
    final: dict = {}

    def merge(*groups):
        kept = {}
        for group in groups:
            for index, time, gain in group:
                key = (index, round(time, 3))
                if key not in kept or gain > kept[key][2]:
                    kept[key] = (index, time, gain)
        return list(kept.values())

    class Tracked(np.ndarray):
        """An audio array that remembers which notes sound in it, and when."""

        def __array_finalize__(self, obj):
            self.marks = list(getattr(obj, "marks", ()))
            self.sr = getattr(obj, "sr", None)
            self.parent = None
            self.offset = 0

        def __getitem__(self, key):
            result = super().__getitem__(key)
            if not isinstance(result, Tracked):
                return result
            first = key[0] if isinstance(key, tuple) and key else key
            if isinstance(first, slice) and self.ndim >= 1:
                start, stop, step = first.indices(self.shape[0])
                if step == 1:
                    rate = self.sr or 44100
                    low, high = start / rate, stop / rate
                    result.marks = [(i, t - low, g) for i, t, g in self.marks
                                    if low - 1e-9 <= t < high]
                    result.parent, result.offset = self, start
                    return result
            result.marks = []
            return result

        def __setitem__(self, key, value):
            super().__setitem__(key, value.view(np.ndarray) if isinstance(value, Tracked) else value)
            # An in-place edit through a slice view has already reported its notes.
            if isinstance(value, Tracked) and value.marks and value.parent is not self:
                first = key[0] if isinstance(key, tuple) and key else key
                start = first.indices(self.shape[0])[0] if isinstance(first, slice) else 0
                add_marks(self, value.marks, start, value.sr)

        def __array_ufunc__(self, ufunc, method, *inputs, out=None, **kwargs):
            plain = [x.view(np.ndarray) if isinstance(x, Tracked) else x for x in inputs]
            if out is not None:
                kwargs["out"] = tuple(o.view(np.ndarray) if isinstance(o, Tracked) else o
                                      for o in out)
            result = getattr(ufunc, method)(*plain, **kwargs)
            if (method != "__call__" or ufunc.nout != 1 or not isinstance(result, np.ndarray)
                    or result.ndim == 0):
                return result
            tracked = [x for x in inputs if isinstance(x, Tracked)]
            rate = next((x.sr for x in tracked if x.sr), None)
            scale = 1.0
            scalars = [float(abs(x)) for x in inputs
                       if not isinstance(x, np.ndarray) and np.isscalar(x)
                       and isinstance(x, (int, float, np.number))]
            if ufunc is np.multiply:
                scale = math.prod(scalars) if scalars else 1.0
            elif ufunc is np.true_divide and len(inputs) == 2 and scalars and inputs[1] is not inputs[0]:
                if not isinstance(inputs[1], np.ndarray) and scalars[-1] > 0:
                    scale = 1.0 / scalars[-1]
            if out is not None and isinstance(out[0], Tracked):
                target = out[0]
                if scale != 1.0:
                    target.marks = [(i, t, g * scale) for i, t, g in target.marks]
                others = [m for x in tracked if x is not target for m in x.marks]
                if others:
                    add_marks(target, others, 0, rate)
                return target
            marks = merge(*(x.marks for x in tracked))
            if scale != 1.0:
                marks = [(i, t, g * scale) for i, t, g in marks]
            return tag(result, marks, rate)

    def tag(array, marks, rate):
        result = np.asarray(array).view(Tracked)
        result.marks, result.sr = list(marks), rate
        return result

    def marks_of(value):
        return value.marks if isinstance(value, Tracked) else []

    def add_marks(target, marks, start_frames, rate_hint):
        rate = target.sr or rate_hint or 44100
        target.sr = target.sr or rate_hint
        relative = [(i, t + start_frames / rate, g) for i, t, g in marks]
        target.marks = merge(target.marks, relative)
        node = target
        while node.parent is not None:
            parent = node.parent
            parent.sr = parent.sr or rate
            relative = [(i, t + node.offset / parent.sr, g) for i, t, g in relative]
            parent.marks = merge(parent.marks, relative)
            node = parent

    def bound(function, args, kwargs):
        call = inspect.signature(function).bind(*args, **kwargs)
        call.apply_defaults()
        values = dict(call.arguments)
        values.update(values.pop("kwargs", None) or {})
        return values

    def register(kind, label, note, duration, volume, rate, array):
        frequency = float(synth.note_to_freq(note)) if kind == "pitched" else 0.0
        if kind == "pitched" and frequency == 0:
            kind = "rest"
        notes.append({"kind": kind, "label": label, "note": str(note)[:16],
                      "frequency": frequency, "duration": float(duration),
                      "volume": float(volume)})
        return tag(array, [(len(notes) - 1, 0.0, 1.0)], rate)

    timbres: dict = {}

    def synth_note(*args, **kwargs):
        result = synth.synth_note(*args, **kwargs)
        v = bound(synth.synth_note, args, kwargs)
        wave = str(v["wave_type"]).lower().replace("-", "_").replace(" ", "_")
        # A waveform alone merged a sawtooth lead with a sawtooth bass. Sustain
        # and release separate authored voices; attack often varies by section.
        key = (wave, round(float(v["sustain"]), 2), round(float(v["release"]), 2))
        if key not in timbres:
            timbres[key] = sum(1 for known in timbres if known[0] == wave) + 1
        return register("noise" if wave in NOISE_WAVES else "pitched", f"synth:{wave}#{timbres[key]}",
                        v["note"], v["duration"], v["volume"], int(v["sr"]), result)

    def generate_wave(*args, **kwargs):
        result = synth.generate_wave(*args, **kwargs)
        v = bound(synth.generate_wave, args, kwargs)
        wave = str(v["wave_type"]).lower().replace("-", "_").replace(" ", "_")
        return register("noise" if wave in NOISE_WAVES else "pitched", f"wave:{wave}",
                        v["freq"], v["duration"], 1.0, int(v["sr"]), result)

    def pink_noise(*args, **kwargs):
        result = synth.pink_noise(*args, **kwargs)
        v = bound(synth.pink_noise, args, kwargs)
        return register("noise", "pink_noise", "noise", v["duration"], v["volume"],
                        int(v["sr"]), result)

    def instrument_note(note, duration, instrument="piano", volume=0.2, sr=44100, **kwargs):
        # Same checks as the real helper, without the native renderer: a quiet
        # sine keeps normalisation arithmetic finite while notes are recorded.
        frames = synth._duration_frames(duration, sr)
        if not 0.001 <= float(duration) <= 30:
            raise ValueError("native instrument note duration must be .001..30 seconds")
        frequency = synth.note_to_freq(note)
        if not math.isfinite(float(volume)) or volume < 0:
            raise ValueError("volume must be finite and nonnegative")
        if frequency and frequency >= sr / 2:
            raise ValueError("frequency must be below the requested sample rate's Nyquist limit")
        t = np.arange(frames) / sr
        signal = (np.sin(2 * np.pi * frequency * t) * np.minimum(1.0, t / 0.005)
                  * np.exp(-2.0 * t) * float(volume))
        return register("pitched", str(instrument), note, duration, volume, int(sr), signal)

    def mix_waves(*args, **kwargs):
        result = synth.mix_waves(*args, **kwargs)
        v = bound(synth.mix_waves, args, kwargs)
        start, gain = float(v["start_time"]), float(v["volume_b"])
        placed = [(i, t + start, g * gain) for i, t, g in marks_of(v["wave_b"]) if t + start >= -1e-9]
        rate = getattr(v["wave_a"], "sr", None) or getattr(v["wave_b"], "sr", None) or int(v["sr"])
        return tag(result, merge(marks_of(v["wave_a"]), placed), rate)

    def preserving(name):
        function = getattr(synth, name)

        def call(*args, **kwargs):
            result = function(*args, **kwargs)
            source = args[0] if args else kwargs.get("wave")
            if not isinstance(result, np.ndarray):
                return result
            marks, rate = marks_of(source), getattr(source, "sr", None)
            if name == "fit_to_length" and marks:
                limit = len(result) / (rate or 44100)
                marks = [m for m in marks if m[1] < limit]
            return tag(result, marks, rate)
        return call

    def play_sound(*args, **kwargs):
        wave = args[0] if args else kwargs.get("wave")
        final["marks"] = marks_of(wave)
        return synth.play_sound(*args, **kwargs)

    sequence_joins = {"concatenate", "hstack"}
    constructors = {"zeros", "ones", "full", "zeros_like", "ones_like", "full_like"}

    def tracked_inputs(values):
        for value in values:
            if isinstance(value, Tracked):
                yield value
            elif isinstance(value, (list, tuple)):
                yield from tracked_inputs(value)

    def tracked_numpy(name, function):
        def call(*args, **kwargs):
            result = function(*args, **kwargs)
            if not isinstance(result, np.ndarray):
                return result
            if name in constructors:
                like = args[0] if name.endswith("_like") and args else None
                return tag(result, [], getattr(like, "sr", None))
            inputs = list(tracked_inputs(list(args) + list(kwargs.values())))
            if not inputs:
                return np.asarray(result)
            rate = next((x.sr for x in inputs if x.sr), None)
            if name in sequence_joins and args and isinstance(args[0], (list, tuple)):
                marks, cursor = [], 0
                for part in args[0]:
                    if isinstance(part, Tracked):
                        marks += [(i, t + cursor / (rate or 44100), g) for i, t, g in part.marks]
                    cursor += len(part) if hasattr(part, "__len__") else 1
                return tag(result, merge(marks), rate)
            if name == "tile" and len(args) >= 2 and isinstance(args[0], Tracked) and np.isscalar(args[1]):
                span = len(args[0]) / (rate or 44100)
                marks = [(i, t + k * span, g) for k in range(int(args[1])) for i, t, g in args[0].marks]
                return tag(result, merge(marks), rate)
            if name == "pad" and args and isinstance(args[0], Tracked):
                width = np.asarray(args[1] if len(args) > 1 else kwargs.get("pad_width", 0), dtype=object)
                before = int(width) if width.ndim == 0 else int(np.ravel(width)[0])
                return tag(result, [(i, t + before / (rate or 44100), g) for i, t, g in args[0].marks], rate)
            if name == "roll" and len(args) >= 2 and isinstance(args[0], Tracked) and np.isscalar(args[1]):
                shift = int(args[1]) / (rate or 44100)
                return tag(result, [(i, t + shift, g) for i, t, g in args[0].marks], rate)
            return tag(result, merge(*(x.marks for x in inputs)), rate)
        return call

    original_modules = worker._safe_modules

    def instrumented_modules():
        modules = original_modules()
        numpy_module = modules["numpy"]
        for name in worker.NUMPY_FUNCTIONS:
            setattr(numpy_module, name, tracked_numpy(name, getattr(numpy_module, name)))
        values = {name: getattr(synth, name) for name in worker.SYNTH_EXPORTS}
        values.update(synth_note=synth_note, instrument_note=instrument_note,
                      generate_wave=generate_wave, pink_noise=pink_noise,
                      mix_waves=mix_waves, play_sound=play_sound,
                      **{name: preserving(name) for name in PRESERVING})
        modules["teledra_synth"] = worker._module("teledra_synth", values)
        return modules

    worker._safe_modules = instrumented_modules
    captured: dict = {}
    synth._capture_callback = lambda array, rate, loop: captured.update(
        frames=len(array), rate=rate, loop=loop)
    synth._configure_bridge(None)
    try:
        tree = worker.validate_source(source)
        namespace = worker._namespace()
        exec(compile(tree, "composition.py", "exec"), namespace, namespace)
    except Exception as error:
        return {"ok": False, "error": f"{type(error).__name__}: {error}", "notes": notes}
    finally:
        synth._capture_callback = None
    if "marks" not in final:
        return {"ok": False, "error": "composition never called play_sound", "notes": notes}
    metadata = namespace.get("TELEDRA_SCORE")
    metadata = metadata if isinstance(metadata, dict) else {}

    def plain(value):
        try:
            return json.loads(json.dumps(value, default=str, allow_nan=False))
        except (TypeError, ValueError):
            return str(value)

    return {
        "ok": True, "notes": notes,
        "marks": [[i, t, g] for i, t, g in sorted(final["marks"], key=lambda m: m[1])],
        "rate": captured.get("rate"), "loop": captured.get("loop"),
        "title": plain(metadata.get("title", namespace.get("TITLE"))),
        "bpm": plain(metadata.get("bpm", namespace.get("BPM"))),
        "key": plain(metadata.get("key", namespace.get("KEY"))),
    }


def run_capture(code: Path) -> dict:
    with tempfile.TemporaryDirectory() as temporary:
        output = Path(temporary) / "capture.json"
        command = [sys.executable, str(Path(__file__).resolve()), "--capture", str(code),
                   "--capture-out", str(output)]
        try:
            completed = subprocess.run(command, cwd=str(HERE), capture_output=True, text=True,
                                       timeout=CAPTURE_TIMEOUT)
        except subprocess.TimeoutExpired:
            return {"ok": False, "error": f"note capture exceeded {CAPTURE_TIMEOUT} s"}
        if output.is_file():
            return json.loads(output.read_text(encoding="utf-8"))
        return {"ok": False, "error": (completed.stderr or "note capture wrote nothing").strip()[-600:]}


# -------------------------------------------------------------- symbolic ----

def name_of(midi: int) -> str:
    return f"{NOTE_NAMES[midi % 12]}{midi // 12 - 1}"


def events_from(capture_result: dict, duration: float) -> list[dict]:
    notes = capture_result.get("notes", [])
    events = []
    for index, time, gain in capture_result.get("marks", []):
        note = notes[int(index)]
        if note["kind"] == "rest" or time >= duration:
            continue
        midi = None
        if note["kind"] == "pitched" and note["frequency"] > 0:
            midi = int(round(69 + 12 * math.log2(note["frequency"] / 440.0)))
        start = max(0.0, float(time))
        events.append({"t": start, "end": min(duration, start + note["duration"]), "midi": midi,
                       "label": note["label"], "level": float(gain) * note["volume"],
                       "source": int(index)})
    events.sort(key=lambda e: (e["t"], -(e["midi"] if e["midi"] is not None else -1)))
    return events


def events_from_audit(audit: list[dict], duration: float) -> list[dict]:
    """Notes from the worker's composition audit, used by song-recipe renders."""
    events = []
    for index, item in enumerate(audit):
        start = float(item.get("start_seconds", 0.0))
        if not item.get("mixed", True) or start >= duration:
            continue
        midi = item.get("midi")
        events.append({"t": start,
                       "end": min(duration, start + float(item.get("duration_seconds", 0.0))),
                       "midi": int(midi) if midi is not None else None,
                       "label": f"{item.get('layer', 'part')}:{item.get('voice', '?')}",
                       "level": float(item.get("velocity", 1.0)),
                       "source": int(item.get("id", index))})
    events.sort(key=lambda e: (e["t"], -(e["midi"] if e["midi"] is not None else -1)))
    return events


MELODY_WORDS = ("melody", "theme", "answer", "lead", "motif")


def event_keys(events: list[dict]) -> list[str]:
    """Compact note identities for comparing pieces: onset, pitch and a melody flag."""
    return [f"{e['t']:.1f}|{e['midi']}|{int(any(w in e['label'].lower() for w in MELODY_WORDS))}"
            for e in events if e["midi"] is not None and e["level"] > 1e-5]


def identity(before: list[str], after: list[str]) -> float | None:
    """Share of note events two pieces have in common, 1.0 meaning the same notes."""
    if not before and not after:
        return None
    shared = sum((Counter(before) & Counter(after)).values())
    return round(shared / max(len(before), len(after)), 3)


def estimate_key(histogram) -> dict | None:
    histogram = np.asarray(histogram, dtype=float)
    if histogram.sum() <= 0:
        return None
    best = None
    for mode, profile in (("major", MAJOR_PROFILE), ("minor", MINOR_PROFILE)):
        for tonic in range(12):
            with np.errstate(all="ignore"):
                r = float(np.corrcoef(np.roll(profile, tonic), histogram)[0, 1])
            if math.isfinite(r) and (best is None or r > best[0]):
                best = (r, tonic, mode)
    if best is None:
        return None
    r, tonic, mode = best
    degrees = MAJOR_DEGREES if mode == "major" else MINOR_DEGREES
    return {"tonic": NOTE_NAMES[tonic], "mode": mode, "correlation": round(r, 3),
            "scale": sorted((tonic + d) % 12 for d in degrees)}


def line_stats(pitches: list[int]) -> dict:
    intervals = [b - a for a, b in zip(pitches, pitches[1:])]
    best = run = 1
    previous = 0
    for interval in intervals:
        if 1 <= abs(interval) <= 2:
            sign = 1 if interval > 0 else -1
            run = run + 1 if sign == previous else 2
            previous = sign
        else:
            run, previous = 1, 0
        best = max(best, run)
    count = len(intervals) or 1
    trigrams = [tuple(intervals[i:i + 3]) for i in range(len(intervals) - 2)]
    return {
        "notes": len(pitches),
        "step_share": round(sum(1 <= abs(i) <= 2 for i in intervals) / count, 3),
        "repeat_share": round(sum(i == 0 for i in intervals) / count, 3),
        "leap_share": round(sum(abs(i) >= 5 for i in intervals) / count, 3),
        "large_leap_share": round(sum(abs(i) >= 8 for i in intervals) / count, 3),
        "longest_scale_run": best if pitches else 0,
        "distinct_interval_patterns": round(len(set(trigrams)) / len(trigrams), 3) if trigrams else None,
        "interval_patterns": sorted({",".join(map(str, g)) for g in trigrams}),
    }


def top_line(events: list[dict]) -> list[int]:
    """The note on top of the texture, recorded each time a new note takes the top.

    Taking the highest note that starts at each onset put the bass into the
    melody whenever the bass entered alone under a held tune.
    """
    line = []
    for onset in sorted({round(e["t"], 2) for e in events}):
        probe = onset + 0.006
        sounding = [e for e in events if e["t"] <= probe < e["end"]]
        if not sounding:
            continue
        top = max(sounding, key=lambda e: (e["midi"], e["t"]))
        if abs(top["t"] - onset) <= 0.006:
            line.append(top["midi"])
    return line


def cosine_distance(a, b) -> float | None:
    if a is None or b is None:
        return None
    a, b = np.asarray(a, dtype=float), np.asarray(b, dtype=float)
    norm = float(np.linalg.norm(a) * np.linalg.norm(b))
    return None if norm == 0 else round(1.0 - float(np.dot(a, b)) / norm, 3)


def symbolic_metrics(events: list[dict], duration: float) -> dict:
    audible = [e for e in events if e["level"] > 1e-5]
    pitched = [e for e in audible if e["midi"] is not None]
    noise = [e for e in audible if e["midi"] is None]
    result = {"placed_events": len(events), "audible_events": len(audible),
              "pitched_notes": len(pitched), "noise_events": len(noise),
              "inaudible_placements": len(events) - len(audible)}
    if not pitched or duration <= 0:
        return result
    labels = Counter(e["label"] for e in audible)
    histogram = np.zeros(12)
    for e in pitched:
        histogram[e["midi"] % 12] += max(0.01, e["end"] - e["t"])
    key = estimate_key(histogram)
    in_key = (sum(e["midi"] % 12 in key["scale"] for e in pitched) / len(pitched)) if key else None

    resolution = 0.02
    bins = int(math.ceil(duration / resolution)) + 1
    change = np.zeros(bins + 1)
    for e in pitched:
        change[int(e["t"] / resolution)] += 1
        change[max(int(e["t"] / resolution) + 1, int(e["end"] / resolution))] -= 1
    active = np.cumsum(change)[:bins]
    sounding = active > 0

    onsets = sorted({round(e["t"], 2) for e in pitched})
    gaps = [round(b - a, 2) for a, b in zip(onsets, onsets[1:])]
    gap_counts = Counter(gaps)
    dissonant = pairs = 0
    for onset in onsets:
        heard = sorted({e["midi"] for e in pitched if e["t"] <= onset + 1e-6 < e["end"]})
        for i, low in enumerate(heard):
            for high in heard[i + 1:]:
                pairs += 1
                dissonant += (high - low) % 12 in (1, 6, 11)

    quarters, quarter_counts = [], []
    for q in range(4):
        low, high = q * duration / 4, (q + 1) * duration / 4
        chosen = [e for e in pitched if low <= e["t"] < high]
        vector = np.zeros(12)
        for e in chosen:
            vector[e["midi"] % 12] += max(0.01, e["end"] - e["t"])
        quarters.append(vector)
        quarter_counts.append(len(chosen))
    contrasts = [d for d in (cosine_distance(a, b) for a, b in zip(quarters, quarters[1:])) if d is not None]

    melody = top_line(pitched)
    voices = {}
    for label in sorted({e["label"] for e in pitched}):
        mine = [e for e in pitched if e["label"] == label]
        line = top_line(mine)
        stats = line_stats(line)
        stats.pop("interval_patterns")
        voices[label] = {"notes": len(mine), "range": [name_of(min(e["midi"] for e in mine)),
                                                       name_of(max(e["midi"] for e in mine))],
                         "mean_duration": round(float(np.mean([e["end"] - e["t"] for e in mine])), 3),
                         **stats}
    result.update({
        "instruments": dict(labels.most_common()),
        "distinct_pitches": len({e["midi"] for e in pitched}),
        "distinct_pitch_classes": int(np.count_nonzero(histogram)),
        "range": [name_of(min(e["midi"] for e in pitched)), name_of(max(e["midi"] for e in pitched))],
        "pitch_class_histogram": [round(float(v), 3) for v in histogram / histogram.sum()],
        "key_estimate": key,
        "in_key_share": round(in_key, 3) if in_key is not None else None,
        "notes_per_second": round(len(pitched) / duration, 2),
        "silence_share": round(1.0 - float(np.mean(sounding)), 3),
        "max_polyphony": int(active.max()),
        "mean_polyphony_while_sounding": round(float(active[sounding].mean()), 2) if sounding.any() else 0,
        "distinct_onsets": len(onsets),
        "distinct_onset_gaps": len(gap_counts),
        "most_common_gap": ({"seconds": gap_counts.most_common(1)[0][0],
                             "share": round(gap_counts.most_common(1)[0][1] / len(gaps), 3)}
                            if gaps else None),
        "dissonant_pair_share": round(dissonant / pairs, 3) if pairs else None,
        "simultaneous_pairs": pairs,
        "quarter_note_counts": quarter_counts,
        "quarter_pitch_contrast": round(float(np.mean(contrasts)), 3) if contrasts else None,
        "return_similarity": (round(1.0 - cosine_distance(quarters[0], quarters[3]), 3)
                              if cosine_distance(quarters[0], quarters[3]) is not None else None),
        "melody": {"onsets": len(melody), "opening": [name_of(m) for m in melody[:16]],
                   **line_stats(melody)},
        "voices": voices,
        "events_preview": [{"t": round(e["t"], 3), "note": name_of(e["midi"]),
                            "dur": round(e["end"] - e["t"], 3), "label": e["label"]}
                           for e in pitched[:40]],
    })
    return result


def source_checks(text: str) -> dict:
    sys.path.insert(0, str(HERE))
    import python_music_worker as worker
    try:
        tree = ast.parse(text)
    except SyntaxError as error:
        return {"parse_error": str(error)}
    exports = set(worker.SYNTH_EXPORTS)
    shadowed = {node.name for node in ast.walk(tree)
                if isinstance(node, ast.FunctionDef) and node.name in exports}
    shadowed |= {target.id for node in ast.walk(tree) if isinstance(node, ast.Assign)
                 for target in node.targets if isinstance(target, ast.Name) and target.id in exports}
    calls = Counter(node.func.id for node in ast.walk(tree)
                    if isinstance(node, ast.Call) and isinstance(node.func, ast.Name))
    return {"lines": len(text.splitlines()),
            "shadowed_library_helpers": sorted(shadowed),
            "helper_call_sites": {name: calls[name] for name in sorted(exports) if calls[name]},
            "loops": sum(isinstance(node, (ast.For, ast.While)) for node in ast.walk(tree))}


# ----------------------------------------------------------------- audio ----

def audio_metrics(path: Path) -> dict:
    import soundfile
    samples, rate = soundfile.read(str(path), always_2d=True)
    mono = samples.mean(axis=1)
    duration = len(mono) / rate
    peak = float(np.max(np.abs(samples))) if samples.size else 0.0
    rms = float(np.sqrt(np.mean(mono ** 2))) if mono.size else 0.0
    result = {"seconds": round(duration, 3), "sample_rate": int(rate), "channels": samples.shape[1],
              "peak": round(peak, 4), "rms_dbfs": round(20 * math.log10(max(rms, 1e-9)), 2),
              "crest_db": round(20 * math.log10(max(peak, 1e-9) / max(rms, 1e-9)), 2),
              "clipped_share": round(float(np.mean(np.abs(samples) >= 0.999)), 5)}
    if samples.shape[1] == 2:
        side = (samples[:, 0] - samples[:, 1]) / 2
        result["stereo_side_to_mid"] = round(float(np.sqrt(np.mean(side ** 2))) / max(rms, 1e-9), 3)
    # A 50 ms edge flagged every piece, including ones that loop well. Compare the
    # last and first two seconds, floored at -60 dBFS so silence stays finite.
    edge = max(1, min(len(mono) // 4, int(2.0 * rate)))
    head = max(1e-3, float(np.sqrt(np.mean(mono[:edge] ** 2))))
    tail = max(1e-3, float(np.sqrt(np.mean(mono[-edge:] ** 2))))
    result["loop_seam"] = {"level_step_db": round(abs(20 * math.log10(tail / head)), 2),
                           "sample_jump": round(float(np.max(np.abs(samples[-1] - samples[0]))), 4)}
    try:
        import librosa
    except Exception as error:
        result["librosa_error"] = str(error)
        return result
    target = 22050
    y = librosa.resample(mono.astype(np.float32), orig_sr=rate, target_sr=target)
    hop = 512
    frame_rms = librosa.feature.rms(y=y, frame_length=2048, hop_length=hop)[0]
    level_db = 20 * np.log10(np.maximum(frame_rms, 1e-9))
    minimum_gap = max(1, int(round(0.1 / (hop / target))))
    gaps = run = 0
    for quiet in level_db < -50:
        run = run + 1 if quiet else 0
        gaps += run == minimum_gap
    envelope = librosa.onset.onset_strength(y=y, sr=target, hop_length=hop)
    onsets = librosa.onset.onset_detect(onset_envelope=envelope, sr=target, hop_length=hop, units="time")
    tempo, beats = librosa.beat.beat_track(onset_envelope=envelope, sr=target, hop_length=hop)
    chroma = librosa.feature.chroma_stft(y=y, sr=target, hop_length=hop)
    chroma_mean = chroma.mean(axis=1)
    normalised = chroma_mean / max(float(chroma_mean.sum()), 1e-9)
    entropy = float(-np.sum(normalised * np.log2(np.maximum(normalised, 1e-12))))
    result.update({
        "silence_share": round(float(np.mean(level_db < -50)), 3),
        "silent_gaps": int(gaps),
        "seconds_per_gap": round(duration / gaps, 2) if gaps else None,
        "dynamic_range_db": round(float(np.percentile(level_db, 95) - np.percentile(level_db, 10)), 2),
        "spectral_centroid_hz": round(float(librosa.feature.spectral_centroid(y=y, sr=target, hop_length=hop).mean()), 1),
        "spectral_flatness": round(float(librosa.feature.spectral_flatness(y=y, hop_length=hop).mean()), 4),
        "onsets_per_second": round(len(onsets) / max(duration, 1e-9), 2),
        "tempo_estimate_bpm": round(float(np.atleast_1d(tempo)[0]), 1),
        "beats_found": int(len(beats)),
        "chroma_mean": [round(float(v), 4) for v in chroma_mean],
        "chroma_entropy_bits": round(entropy, 3),
        "audio_key_estimate": estimate_key(chroma_mean),
    })
    step = max(1, int(0.25 * target / hop))
    frames = chroma[:, ::step]
    if frames.shape[1] >= 16:
        unit = frames / np.maximum(np.linalg.norm(frames, axis=0, keepdims=True), 1e-9)
        similarity = unit.T @ unit
        lag = max(1, int(2.0 / 0.25))
        repeats = [float(np.max(np.concatenate([similarity[i, :max(0, i - lag + 1)],
                                                similarity[i, i + lag:]]) if similarity.shape[1] > lag else [0]))
                   for i in range(similarity.shape[0])]
        result["harmonic_repetition_share"] = round(float(np.mean(np.array(repeats) >= 0.97)), 3)
    result["_plot"] = {"level_db": level_db.tolist(), "hop_seconds": hop / target,
                       "onsets": [float(t) for t in onsets]}
    return result


# ----------------------------------------------------------------- plots ----

def plot(events: list[dict], wav: Path | None, audio: dict, title: str, output: Path) -> None:
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    rows = 3 if wav is not None else 1
    figure, axes = plt.subplots(rows, 1, figsize=(14, 3.6 * rows), squeeze=False)
    roll = axes[0][0]
    pitched = [e for e in events if e["midi"] is not None and e["level"] > 1e-5]
    colours = plt.get_cmap("tab10")
    labels = sorted({e["label"] for e in pitched})
    for index, label in enumerate(labels):
        mine = [e for e in pitched if e["label"] == label]
        roll.broken_barh([(e["t"], max(0.02, e["end"] - e["t"])) for e in mine],
                         (0, 0), facecolors=colours(index % 10))
        for e in mine:
            roll.broken_barh([(e["t"], max(0.02, e["end"] - e["t"]))], (e["midi"] - 0.4, 0.8),
                             facecolors=colours(index % 10), alpha=0.85)
        roll.plot([], [], color=colours(index % 10), linewidth=6, label=label)
    if pitched:
        low = min(e["midi"] for e in pitched) - 2
        high = max(e["midi"] for e in pitched) + 2
        roll.set_ylim(low, high)
        ticks = [m for m in range(low, high + 1) if m % 12 in (0, 7)]
        roll.set_yticks(ticks, [name_of(m) for m in ticks])
        roll.legend(loc="upper right", fontsize=8, ncol=2)
    else:
        roll.text(0.5, 0.5, "no pitched notes captured", ha="center", va="center", transform=roll.transAxes)
    roll.set_title(f"{title} - piano roll (captured notes)")
    roll.set_xlabel("seconds")
    roll.grid(alpha=0.3)
    if wav is not None:
        import librosa
        import librosa.display
        import soundfile
        samples, rate = soundfile.read(str(wav), always_2d=True)
        y = librosa.resample(samples.mean(axis=1).astype(np.float32), orig_sr=rate, target_sr=22050)
        mel = librosa.power_to_db(librosa.feature.melspectrogram(y=y, sr=22050, n_mels=96), ref=np.max)
        librosa.display.specshow(mel, sr=22050, x_axis="time", y_axis="mel", ax=axes[1][0])
        axes[1][0].set_title("mel spectrogram (dB)")
        data = audio.get("_plot")
        if data:
            times = np.arange(len(data["level_db"])) * data["hop_seconds"]
            axes[2][0].plot(times, data["level_db"], linewidth=0.8)
            for onset in data["onsets"]:
                axes[2][0].axvline(onset, color="tab:red", alpha=0.25, linewidth=0.6)
            axes[2][0].set_ylim(-80, 0)
        axes[2][0].set_title("loudness (dBFS) with detected onsets")
        axes[2][0].set_xlabel("seconds")
        axes[2][0].grid(alpha=0.3)
        for axis in (roll, axes[1][0], axes[2][0]):
            axis.set_xlim(0, audio.get("seconds") or None)
    figure.tight_layout()
    figure.savefig(output, dpi=90)
    plt.close(figure)


# -------------------------------------------------------- observations ----

def observations(analysis: dict) -> list[str]:
    notes = []
    s, a, c = analysis.get("symbolic", {}), analysis.get("audio", {}), analysis.get("capture", {})
    duration = a.get("seconds") or analysis.get("report", {}).get("duration") or 0
    if analysis.get("report", {}).get("loop") and duration and duration < 15:
        notes.append(f"piece is {duration:.1f} s long and loops, so it repeats every {duration:.1f} s")
    if c and not c.get("ok"):
        notes.append(f"note capture failed ({c.get('error')}); only audio metrics are available")
    coverage = analysis.get("capture_coverage")
    if coverage is not None and coverage < 0.8:
        notes.append(f"only {coverage:.0%} of note helper calls were traced into the final mix; "
                     "symbolic metrics may be incomplete")
    if s.get("pitched_notes") is not None and 0 < s["pitched_notes"] < 16:
        notes.append(f"very few pitched notes ({s['pitched_notes']})")
    melody = s.get("melody", {})
    if melody.get("longest_scale_run", 0) >= 6:
        notes.append(f"top line contains a stepwise scale run of {melody['longest_scale_run']} notes")
    gap = s.get("most_common_gap")
    if gap and s.get("distinct_onsets", 0) > 8 and gap["share"] >= 0.8:
        notes.append(f"{gap['share']:.0%} of onset gaps are {gap['seconds']} s: mechanical rhythm")
    if s.get("max_polyphony") == 1:
        notes.append("only one note sounds at a time: no harmony or accompaniment")
    if s.get("dissonant_pair_share") is not None and s["dissonant_pair_share"] > 0.3:
        notes.append(f"{s['dissonant_pair_share']:.0%} of simultaneous intervals are seconds, sevenths or tritones")
    if s.get("in_key_share") is not None and s["in_key_share"] < 0.8:
        notes.append(f"{1 - s['in_key_share']:.0%} of notes fall outside the estimated key")
    if duration > 10 and s.get("silence_share") is not None and s["silence_share"] < 0.02:
        notes.append("pitched notes never pause: no breathing space")
    if s.get("silence_share") is not None and s["silence_share"] > 0.5:
        notes.append(f"no pitched note sounds for {s['silence_share']:.0%} of the piece")
    if len(s.get("instruments", {})) == 1:
        notes.append("a single timbre carries the whole piece")
    if duration > 30 and s.get("quarter_pitch_contrast") is not None and s["quarter_pitch_contrast"] < 0.05:
        notes.append("the four quarters use nearly the same pitch material: little sectional contrast")
    shadowed = analysis.get("source", {}).get("shadowed_library_helpers")
    if shadowed:
        notes.append(f"source redefines library helper(s) {', '.join(shadowed)}; the sound may not follow the documented behaviour")
    if a.get("clipped_share", 0) > 0.001:
        notes.append(f"{a['clipped_share']:.2%} of samples are clipped")
    if a.get("silence_share", 0) > 0.3:
        notes.append(f"audio is below -50 dBFS for {a['silence_share']:.0%} of frames")
    seam = a.get("loop_seam", {})
    if analysis.get("report", {}).get("loop") and seam.get("level_step_db", 0) > 6:
        notes.append(f"level steps {seam['level_step_db']} dB between the last and first two seconds: "
                     "the loop restart is likely audible")
    if a.get("rms_dbfs") is not None and a["rms_dbfs"] < -26:
        notes.append(f"quiet mix: RMS {a['rms_dbfs']} dBFS")
    if a.get("silent_gaps") and a.get("seconds_per_gap") and a["seconds_per_gap"] < 6:
        notes.append(f"{a['silent_gaps']} silent gaps (100 ms or more below -50 dBFS), "
                     f"one every {a['seconds_per_gap']} s")
    if a.get("stereo_side_to_mid") is not None and a["stereo_side_to_mid"] < 0.02:
        notes.append("effectively mono")
    novelty = analysis.get("novelty")
    if novelty and novelty.get("same_source"):
        notes.append("source is identical to the previous accepted render")
    elif novelty and (novelty.get("pitch_class_distance") or 0) < 0.05 and (novelty.get("melody_pattern_overlap") or 0) > 0.8:
        notes.append("pitch material and melodic patterns nearly match the previous accepted render")
    if novelty and not novelty.get("same_source") and novelty.get("note_identity") is not None:
        if novelty["note_identity"] >= 0.95:
            notes.append(f"shares {novelty['note_identity']:.1%} of its note events with the previous accepted render")
        elif novelty.get("accompaniment_identity") == 1.0:
            notes.append("reuses the previous accepted render's accompaniment note for note")
    nearest = analysis.get("nearest_earlier")
    if nearest and nearest["previous"] != (novelty or {}).get("previous") and (
            nearest.get("same_source")
            or (nearest.get("note_identity") or 0) >= 0.8
            or (nearest.get("accompaniment_identity") == 1.0 and (nearest.get("note_identity") or 0) >= 0.6)
            or ((nearest.get("pitch_class_distance") if nearest.get("pitch_class_distance") is not None else 1) < 0.05
                and (nearest.get("chroma_distance") if nearest.get("chroma_distance") is not None else 1) < 0.05
                and (nearest.get("instrument_overlap") or 0) >= 0.8)):
        notes.append(f"nearly identical to earlier render {nearest['previous']} (note identity "
                     f"{nearest.get('note_identity')}, accompaniment identity {nearest.get('accompaniment_identity')})")
    return notes


# ------------------------------------------------------------- analysis ----

def analyse_render(directory: Path, out_root: Path, force: bool = False) -> dict:
    code = directory / "code.py"
    text = code.read_text(encoding="utf-8-sig") if code.is_file() else ""
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]
    destination = out_root / directory.name
    cached = destination / "analysis.json"
    if cached.is_file() and not force:
        previous = json.loads(cached.read_text(encoding="utf-8"))
        if previous.get("source_sha") == digest and previous.get("harness_version") == HARNESS_VERSION:
            return previous
    destination.mkdir(parents=True, exist_ok=True)
    report_path = directory / "report.json"
    report = json.loads(report_path.read_text(encoding="utf-8")) if report_path.is_file() else {}
    wav = directory / "audio.wav"
    audio = audio_metrics(wav) if wav.is_file() else {}
    duration = audio.get("seconds") or float(report.get("duration") or 0)
    audit = report.get("composition_audit")
    if isinstance(audit, dict) and audit.get("events"):
        # Song-recipe renders go through render_song, which the replay cannot see
        # into; the worker's own audit already lists every placed note.
        metadata = report.get("metadata") or {}
        captured = {"ok": True, "source": "report composition_audit", "title": report.get("title"),
                    "bpm": metadata.get("bpm"), "key": metadata.get("key")}
        events = events_from_audit(audit["events"], duration)
        requested = len(audit["events"])
        placed = sum(1 for item in audit["events"] if item.get("mixed", True))
    else:
        captured = run_capture(code) if text else {"ok": False, "error": "no code.py"}
        captured.setdefault("source", "sandbox replay")
        events = events_from(captured, duration) if captured.get("ok") else []
        requested = sum(1 for n in captured.get("notes", []) if n["kind"] != "rest")
        placed = len({e["source"] for e in events})
    analysis = {
        "harness_version": HARNESS_VERSION, "render": directory.name, "directory": str(directory),
        "source_sha": digest, "report": {k: report.get(k) for k in ("ok", "duration", "loop", "notes_rendered", "error")},
        "capture": {k: captured.get(k) for k in ("ok", "error", "source", "title", "bpm", "key", "rate")},
        "capture_coverage": round(placed / requested, 3) if requested else None,
        "event_keys": event_keys(events),
        "symbolic": symbolic_metrics(events, duration) if events else {},
        "audio": audio, "source": source_checks(text) if text else {},
    }
    title = captured.get("title") or report.get("title") or directory.name
    try:
        plot(events, wav if wav.is_file() else None, audio, f"{directory.name}: {title}", destination / "critic_view.png")
        analysis["plot"] = str(destination / "critic_view.png")
    except Exception as error:
        analysis["plot_error"] = f"{type(error).__name__}: {error}"
    audio.pop("_plot", None)
    analysis["observations"] = observations(analysis)
    (destination / "events.json").write_text(json.dumps(events, indent=1), encoding="utf-8")
    cached.write_text(json.dumps(analysis, indent=1), encoding="utf-8")
    return analysis


def novelty(previous: dict, current: dict) -> dict:
    ps, cs = previous.get("symbolic", {}), current.get("symbolic", {})
    before = set(ps.get("instruments", {}))
    after = set(cs.get("instruments", {}))
    patterns_before = set(ps.get("melody", {}).get("interval_patterns", []))
    patterns_after = set(cs.get("melody", {}).get("interval_patterns", []))
    durations = [previous.get("audio", {}).get("seconds") or 0, current.get("audio", {}).get("seconds") or 0]

    def jaccard(a, b):
        return round(len(a & b) / len(a | b), 3) if a | b else None

    keys_before, keys_after = previous.get("event_keys") or [], current.get("event_keys") or []
    layered = any(key.endswith("|1") for key in keys_before + keys_after)
    return {"previous": previous["render"], "same_source": previous["source_sha"] == current["source_sha"],
            "note_identity": identity(keys_before, keys_after),
            "accompaniment_identity": (identity([k for k in keys_before if k.endswith("|0")],
                                                [k for k in keys_after if k.endswith("|0")])
                                       if layered else None),
            "pitch_class_distance": cosine_distance(ps.get("pitch_class_histogram"), cs.get("pitch_class_histogram")),
            "chroma_distance": cosine_distance(previous.get("audio", {}).get("chroma_mean"),
                                               current.get("audio", {}).get("chroma_mean")),
            "instrument_overlap": jaccard(before, after),
            "melody_pattern_overlap": jaccard(patterns_before, patterns_after),
            "duration_ratio": round(min(durations) / max(durations), 3) if max(durations) else None}


def closeness(comparison: dict) -> float:
    """Lower is more alike; an identical source always ranks first."""
    if comparison.get("same_source"):
        return -1.0
    if comparison.get("note_identity") is not None:
        return 1.0 - comparison["note_identity"]
    distances = (comparison.get("pitch_class_distance"), comparison.get("chroma_distance"))
    return (sum(d if d is not None else 1.0 for d in distances)
            + 1.0 - (comparison.get("instrument_overlap") or 0.0))


def summary(analysis: dict) -> str:
    s, a, c, r = analysis.get("symbolic", {}), analysis.get("audio", {}), analysis.get("capture", {}), analysis.get("report", {})
    lines = [f"== {analysis['render']}  title: {c.get('title') or '-'}  key hint: {c.get('key') or '-'}  bpm hint: {c.get('bpm') or '-'}"]
    if a:
        lines.append(f"audio: {a.get('seconds')} s, loop={r.get('loop')}, peak {a.get('peak')}, rms {a.get('rms_dbfs')} dBFS, "
                     f"crest {a.get('crest_db')} dB, dynamic range {a.get('dynamic_range_db')} dB, silence {a.get('silence_share')}, "
                     f"side/mid {a.get('stereo_side_to_mid')}")
        lines.append(f"       onsets {a.get('onsets_per_second')}/s, tempo ~{a.get('tempo_estimate_bpm')} BPM, "
                     f"centroid {a.get('spectral_centroid_hz')} Hz, audio key {fmt_key(a.get('audio_key_estimate'))}, "
                     f"harmonic repetition {a.get('harmonic_repetition_share')}, loop seam {a.get('loop_seam')}")
    if not c.get("ok"):
        lines.append(f"capture: FAILED {c.get('error')}")
    lines.append(f"capture ({c.get('source') or 'sandbox replay'}): {analysis.get('capture_coverage')} "
                 "of notes traced into the final mix")
    if s.get("pitched_notes"):
        m = s["melody"]
        lines += [
            f"notes: {s['pitched_notes']} pitched, {s['noise_events']} noise, {s['notes_per_second']}/s; "
            f"instruments {s['instruments']}",
            f"pitch: {s['distinct_pitches']} pitches / {s['distinct_pitch_classes']} classes, range {s['range'][0]}..{s['range'][1]}, "
            f"key {fmt_key(s['key_estimate'])}, in key {s['in_key_share']}",
            f"melody: {m['onsets']} onsets, opens {' '.join(m['opening'][:12])}; steps {m['step_share']}, leaps {m['leap_share']}, "
            f"repeats {m['repeat_share']}, longest scale run {m['longest_scale_run']}, distinct patterns {m['distinct_interval_patterns']}",
            f"rhythm: {s['distinct_onsets']} onsets, {s['distinct_onset_gaps']} distinct gaps, most common {s['most_common_gap']}",
            f"texture: max polyphony {s['max_polyphony']}, mean {s['mean_polyphony_while_sounding']}, silence {s['silence_share']}, "
            f"dissonant pairs {s['dissonant_pair_share']} of {s['simultaneous_pairs']}",
            f"form: notes per quarter {s['quarter_note_counts']}, quarter contrast {s['quarter_pitch_contrast']}, "
            f"return similarity {s['return_similarity']}",
        ]
        for label, voice in s["voices"].items():
            lines.append(f"  voice {label}: {voice['notes']} notes {voice['range'][0]}..{voice['range'][1]}, "
                         f"mean dur {voice['mean_duration']} s, steps {voice['step_share']}, leaps {voice['leap_share']}, "
                         f"run {voice['longest_scale_run']}")
    src = analysis.get("source", {})
    if src:
        lines.append(f"source: {src.get('lines')} lines, loops {src.get('loops')}, helper calls {src.get('helper_call_sites')}, "
                     f"shadowed {src.get('shadowed_library_helpers')}")
    if analysis.get("novelty"):
        lines.append(f"vs previous: {analysis['novelty']}")
    if analysis.get("nearest_earlier"):
        lines.append(f"nearest earlier: {analysis['nearest_earlier']}")
    for note in analysis.get("observations", []):
        lines.append(f"  observation: {note}")
    if analysis.get("plot"):
        lines.append(f"plot: {analysis['plot']}")
    return "\n".join(lines)


def fmt_key(key: dict | None) -> str:
    return f"{key['tonic']} {key['mode']} (r={key['correlation']})" if key else "-"


def accepted_renders(limit: int) -> list[Path]:
    home = data_root() / "python"
    found = []
    for directory in home.glob("render-*"):
        report = directory / "report.json"
        if report.is_file() and (directory / "audio.wav").is_file():
            try:
                if json.loads(report.read_text(encoding="utf-8")).get("ok"):
                    found.append((report.stat().st_mtime, directory))
            except (OSError, ValueError):
                continue
    return [directory for _, directory in sorted(found)[-limit:]]


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("renders", nargs="*", type=Path, help="render directories to analyse")
    parser.add_argument("--current", action="store_true", help="analyse the accepted PyMusic render")
    parser.add_argument("--history", type=int, metavar="N", help="analyse the last N accepted renders and compare them")
    parser.add_argument("--wav", type=Path, help="analyse any WAV (for example the native current.wav), audio only")
    parser.add_argument("--out", type=Path, default=None, help="output directory (default: <data>/music/critic)")
    parser.add_argument("--force", action="store_true", help="ignore cached analyses")
    parser.add_argument("--capture", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--capture-out", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if args.capture:
        result = capture(args.capture.read_text(encoding="utf-8-sig"))
        args.capture_out.write_text(json.dumps(result), encoding="utf-8")
        return 0
    out_root = args.out or data_root() / "critic"
    out_root.mkdir(parents=True, exist_ok=True)
    if args.wav:
        audio = audio_metrics(args.wav)
        destination = out_root / args.wav.stem
        destination.mkdir(parents=True, exist_ok=True)
        plot([], args.wav, audio, args.wav.name, destination / "critic_view.png")
        audio.pop("_plot", None)
        (destination / "audio.json").write_text(json.dumps(audio, indent=1), encoding="utf-8")
        print(json.dumps(audio, indent=1))
        print(f"plot: {destination / 'critic_view.png'}")
        return 0
    directories = list(args.renders)
    if args.current:
        pointer = data_root() / "python" / "current.json"
        directories.append(data_root() / "python" / json.loads(pointer.read_text(encoding="utf-8"))["artifact"])
    if args.history:
        directories = accepted_renders(args.history) + [d for d in directories if d not in accepted_renders(args.history)]
    if not directories:
        parser.error("name render directories, or use --current, --history N or --wav")
    analyses = []
    for directory in directories:
        analysis = analyse_render(directory.resolve(), out_root, force=args.force)
        if analyses:
            analysis["novelty"] = novelty(analyses[-1], analysis)
            # A repeat of an older piece hides from a comparison with only the last one.
            analysis["nearest_earlier"] = min((novelty(earlier, analysis) for earlier in analyses),
                                              key=closeness)
            analysis["observations"] = observations(analysis)
        analyses.append(analysis)
        text = summary(analysis)
        (out_root / directory.name / "summary.txt").write_text(text + "\n", encoding="utf-8")
        print(text + "\n")
    if len(analyses) > 1:
        table = ["render                          secs  notes pitches key            same_src pc_dist chroma_dist instr_overlap pattern_overlap note_id accomp_id  nearest_earlier"]
        for analysis in analyses:
            s, n = analysis.get("symbolic", {}), analysis.get("novelty") or {}
            near = analysis.get("nearest_earlier") or {}
            near_text = (f"{near['previous']} (same_src {near.get('same_source')}, note_id {near.get('note_identity')}, "
                         f"accomp_id {near.get('accompaniment_identity')}, pc {near.get('pitch_class_distance')})"
                         if near else "-")
            table.append(f"{analysis['render']:<31} {analysis.get('audio', {}).get('seconds', '-'):>5} "
                         f"{s.get('pitched_notes', 0):>6} {s.get('distinct_pitches', 0):>7} "
                         f"{fmt_key(s.get('key_estimate'))[:14]:<14} {str(n.get('same_source', '-')):>8} "
                         f"{str(n.get('pitch_class_distance', '-')):>7} {str(n.get('chroma_distance', '-')):>11} "
                         f"{str(n.get('instrument_overlap', '-')):>13} {str(n.get('melody_pattern_overlap', '-')):>15} "
                         f"{str(n.get('note_identity', '-')):>7} {str(n.get('accompaniment_identity', '-')):>9}  {near_text}")
        # Keep every run's table; history.txt always holds the latest.
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        for name in (f"history-{stamp}.txt", "history.txt"):
            (out_root / name).write_text("\n".join(table) + "\n", encoding="utf-8")
        print("\n".join(table))
        print(f"\nhistory table: {out_root / f'history-{stamp}.txt'} (latest also in history.txt)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

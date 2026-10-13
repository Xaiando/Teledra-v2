"""Bounded, device-free five-part song rendering through the existing PyMusic voices.

The input is a musical plan, not Python code. The ledger below is created at
actual additive placements; it never trusts user-authored event declarations.
"""
from __future__ import annotations

import hashlib
import math

import numpy as np

TONICS = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B")
MODES = {"major": (0, 2, 4, 5, 7, 9, 11), "minor": (0, 2, 3, 5, 7, 8, 10),
         "dorian": (0, 2, 3, 5, 7, 9, 10)}
ROLES = ("lead", "answer", "harmony", "bass", "texture")
CLASSICS = ("classic:sine", "classic:triangle", "classic:pluck", "classic:bell",
            "classic:pad", "classic:keys", "classic:bass")
PAN = dict(lead=-.18, answer=.24, harmony=-.10, bass=0., texture=.35)
LEVEL = dict(lead=.15, answer=.13, harmony=.055, bass=.14, texture=.045)
CENTERS = dict(lead=69, answer=72, harmony=56, bass=40, texture=77)


def _keys(value, expected, label):
    if not isinstance(value, dict) or set(value) != set(expected):
        raise ValueError(f"{label} requires exactly these keys: {', '.join(expected)}")


def _number(value, low, high, label, integer=False):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f"{label} must be a finite number")
    if integer and not isinstance(value, int):
        raise ValueError(f"{label} must be an integer")
    if not low <= value <= high:
        raise ValueError(f"{label} must be in {low}..{high}")


def _name(value, label):
    if not isinstance(value, str) or not value.strip() or len(value) > 120:
        raise ValueError(f"{label} must contain 1..120 characters")


def _motif(events, name):
    if not isinstance(events, list) or not 4 <= len(events) <= 10:
        raise ValueError(f"{name} needs 4..10 timed events over eight beats")
    previous_end, gaps = 0., []
    for index, event in enumerate(events):
        _keys(event, ("beat", "degree", "length", "velocity"), f"{name}[{index}]")
        for key, low, high in (("beat", 0, 7.5), ("degree", 0, 13), ("length", .25, 2), ("velocity", .35, 1)):
            _number(event[key], low, high, f"{name}[{index}].{key}", integer=key == "degree")
        if event["beat"] < previous_end - 1e-9:
            raise ValueError(f"{name}[{index}] overlaps or precedes the prior event; write chronological monophonic phrases")
        gaps.append(event["beat"] - previous_end)
        previous_end = event["beat"] + event["length"]
    if previous_end > 7.75 + 1e-9 or max(gaps[1:]) < .2 - 1e-9:
        raise ValueError(f"{name} needs an internal breath of at least .2 beats and must finish by beat 7.75")
    if sum(event["length"] for event in events) > 6.8 + 1e-9:
        raise ValueError(f"{name} fills too much of its phrase; leave at least 1.2 beats of total rest")
    degrees = [event["degree"] for event in events]
    intervals = np.diff(degrees)
    scalar_run = any(np.all(intervals[index:index + 4] == direction)
                     for index in range(len(intervals) - 3) for direction in (-1, 1))
    if len(set(degrees)) < 3 or scalar_run or np.all(intervals == 1) or np.all(intervals == -1):
        raise ValueError(f"{name} needs a shaped motif, not one repeated pitch or an ascending/descending scalar exercise")
    lengths = {round(event["length"], 6) for event in events}
    spacing = {round(events[index + 1]["beat"] - event["beat"], 6) for index, event in enumerate(events[:-1])}
    if len(lengths) == len(spacing) == 1:
        raise ValueError(f"{name} needs rhythmic contrast in event lengths or spacing")


def validate_song(song, native_ids=()):
    """Validate every bounded input before scheduling or synthesizing any note."""
    _keys(song, ("title", "tonic", "mode", "bpm", "progression", "voices", "theme", "answer", "sections", "pulse"), "SONG")
    _name(song["title"], "title")
    if not isinstance(song["tonic"], str) or not isinstance(song["mode"], str) or song["tonic"] not in TONICS or song["mode"] not in MODES:
        raise ValueError("tonic/mode must use the supported pitch names and major/minor/dorian")
    _number(song["bpm"], 72, 132, "bpm")
    progression = song["progression"]
    if not isinstance(progression, list) or len(progression) != 4:
        raise ValueError("progression requires four scale degrees")
    for degree in progression:
        _number(degree, 0, 6, "progression degree", integer=True)
    if len(set(progression)) < 2:
        raise ValueError("progression must move through at least two roots")
    _keys(song["voices"], ROLES, "voices")
    available = set(CLASSICS) | set(native_ids)
    for role, voice in song["voices"].items():
        if not isinstance(voice, str) or voice not in available:
            raise ValueError(f"unsupported {role} voice {voice!r}; select an exact native catalog ID or supported classic recipe")
    _motif(song["theme"], "theme")
    _motif(song["answer"], "answer")
    if [(e["beat"], e["degree"], e["length"]) for e in song["theme"]] == [(e["beat"], e["degree"], e["length"]) for e in song["answer"]]:
        raise ValueError("answer must change pitch or rhythm, not only the theme's velocity")
    sections = song["sections"]
    if not isinstance(sections, list) or len(sections) != 4:
        raise ValueError("sections requires four complete named sections")
    for index, section in enumerate(sections):
        _keys(section, ("name", "bars", "melody", "energy"), f"section {index}")
        _name(section["name"], "section name")
        if type(section["bars"]) is not int or section["bars"] not in (4, 8):
            raise ValueError("each section needs four or eight bars")
        if section["melody"] not in ("none", "theme", "answer", "variation"):
            raise ValueError("section melody must be none/theme/answer/variation")
        _number(section["energy"], .35, 1, "section energy")
    if len({(s["bars"], s["melody"], s["energy"]) for s in sections}) < 2:
        raise ValueError("section names alone are not contrast; change melody, bars or energy")
    if len({section["energy"] for section in sections}) < 2:
        raise ValueError("sections need a dynamic arc; vary at least one section energy")
    melodies = {s["melody"] for s in sections}
    if sum(s["melody"] != "none" for s in sections) < 2 or "theme" not in melodies or not melodies.intersection(("answer", "variation")):
        raise ValueError("the form needs at least two foreground sections, a theme and an answer or variation")
    if song["pulse"] not in ("soft", "syncopated", "flowing"):
        raise ValueError("pulse must be soft, syncopated or flowing")
    duration = sum(section["bars"] for section in sections) * 4 * 60 / song["bpm"]
    if duration < 30:
        raise ValueError("a complete song must last at least 30 seconds; lengthen a section or use BPM <=128 for sixteen bars")


def _midi(degree, tonic, scale, base=60):
    octave, pitch = divmod(int(degree), 7)
    return base + tonic + 12 * octave + scale[pitch]


def _landing(degree, root):
    candidates = [root + interval + 7 * octave for octave in range(-3, 5) for interval in (0, 2, 4)]
    return min(candidates, key=lambda pitch: (abs(pitch - degree), pitch))


def plan_song(song, catalog=()):
    """Derive a deterministic complete arrangement; all resulting events are rendered."""
    validate_song(song, (entry["id"] for entry in catalog))
    beat_seconds = 60 / song["bpm"]
    tonic, scale = TONICS.index(song["tonic"]), MODES[song["mode"]]
    events, sections = [], []
    start_bar = 0

    def event(role, section, beat, degree, length, velocity, provenance):
        events.append(dict(layer=role, section=section, start_beat=beat, length_beats=length,
                           midi=_midi(degree, tonic, scale), velocity=velocity,
                           source=provenance))

    for section_index, section in enumerate(song["sections"]):
        end_bar = start_bar + section["bars"]
        energy = section["energy"]
        sections.append(dict(name=section["name"], bars=section["bars"], melody=section["melody"],
                             energy=energy, start_seconds=start_bar * 4 * beat_seconds,
                             end_seconds=end_bar * 4 * beat_seconds))
        for local_bar in range(section["bars"]):
            bar = start_bar + local_bar
            root = song["progression"][local_bar % 4]
            # Three held harmony tones and an independently articulated bass.
            # The final bar leaves room before the next chapter/loop boundary.
            harmony_length = 3.35 if local_bar != section["bars"] - 1 else 2.9
            for interval in (0, 2, 4):
                event("harmony", section_index, bar * 4 + .08, root + interval - 7, harmony_length, energy * .72, "chord bed")
            bass_beats = (0, 2.5) if song["pulse"] == "syncopated" else (0, 2)
            for index, offset in enumerate(bass_beats):
                event("bass", section_index, bar * 4 + offset, root - 14 + (4 if index else 0),
                      1.15 if index else 1.55, energy * (.85 if index == 0 else .58), "independent bass pulse")
            # The texture enters after the opening bar and rests at chapter ends.
            if (section_index > 0 or local_bar > 0) and local_bar != section["bars"] - 1:
                offsets = (1.5,) if song["pulse"] == "soft" else (1.5, 3.) if song["pulse"] == "syncopated" else (.5, 1.5, 2.5)
                for index, offset in enumerate(offsets):
                    event("texture", section_index, bar * 4 + offset, root + (4, 2, 0)[index % 3] + 7,
                          .45, energy * .55, "offbeat chord colour")
        if section["melody"] != "none":
            for phrase_index in range(section["bars"] // 2):
                response = (phrase_index % 2 == 1) != (section["melody"] == "answer")
                role = "answer" if response else "lead"
                pattern_name = "answer" if response else "theme"
                pattern = song[pattern_name]
                for index, note in enumerate(pattern):
                    beat = start_bar * 4 + phrase_index * 8 + note["beat"]
                    root = song["progression"][int(beat // 4 - start_bar) % 4]
                    degree = note["degree"]
                    # Variations retain the rhythm and motif, changing weak-beat
                    # approach notes before a chord-tone landing.
                    if section["melody"] == "variation" and index % 2 == 1:
                        degree += -1 if index % 4 == 1 else 1
                    if abs(beat % 2) < 1e-8 or index == len(pattern) - 1:
                        degree = _landing(degree, root)
                    event(role, section_index, beat, degree, note["length"],
                          energy * note["velocity"], f"{pattern_name}[{index}]")
        start_bar = end_bar
    # One octave shift per complete part avoids per-note register folding and
    # its invented contour reversals. Native metadata sets the comfortable range.
    ranges = {entry["id"]: entry["range"] for entry in catalog}
    for role in ROLES:
        part = [item for item in events if item["layer"] == role]
        pitches = [item["midi"] for item in part]
        low, high = ranges.get(song["voices"][role], (CENTERS[role] - 14, CENTERS[role] + 14))
        center = min(high, max(low, CENTERS[role]))
        shift = min(range(-60, 61, 12), key=lambda shift: (
            sum(max(0, low - p - shift, p + shift - high) ** 2 for p in pitches),
            abs(np.mean(pitches) + shift - center)))
        for item in part:
            item["midi"] += shift
    for index, item in enumerate(events):
        item.update(id=index, start_seconds=item["start_beat"] * beat_seconds,
                    duration_seconds=item["length_beats"] * beat_seconds)
    return events, sections


def _classic(synth, name, frequency, duration, velocity, sr):
    # Explicit V1 waveform/envelope recipes, not unsupported waveform aliases.
    recipes = {
        "sine": ("sine", .025, .12, .70, .18),
        "triangle": ("triangle", .015, .12, .60, .15),
        "pluck": ("triangle", .006, .16, .15, .09),
        "bell": ("sine", .006, .20, .20, .18),
        "pad": ("sine", .12, .18, .78, .25),
        "keys": ("triangle", .008, .20, .30, .15),
        "bass": ("sine", .008, .10, .62, .12),
    }
    wave, attack, decay, sustain, release = recipes[name]
    result = synth.synth_note(frequency, duration, wave_type=wave, attack=attack,
                             decay=decay, sustain=sustain, release=release, volume=velocity, sr=sr)
    if name in ("bell", "keys", "pad"):
        harmonic, level = (3, .14) if name == "bell" else (2, .12) if name == "keys" else (2, .10)
        if frequency * harmonic < sr / 2:
            colour = synth.synth_note(frequency * harmonic, duration, wave_type="sine", attack=attack,
                                      decay=decay * .7, sustain=sustain * .4, release=release,
                                      volume=velocity * level, sr=sr)
            result = result + colour
    return result


def audio_identity(array):
    return hashlib.sha256(np.ascontiguousarray(array, dtype="<f8").tobytes()).hexdigest()


def render(song, sr, synth):
    _number(sr, 8000, 48000, "song sample rate", integer=True)
    voices = song.get("voices", {}) if isinstance(song, dict) else {}
    native_needed = isinstance(voices, dict) and any(isinstance(voice, str) and not voice.startswith("classic:") for voice in voices.values())
    catalog = synth.instruments() if native_needed else ()
    events, sections = plan_song(song, catalog)
    duration = sections[-1]["end_seconds"]
    frames = int(duration * sr)
    layers = {role: np.zeros(frames) for role in ROLES}
    # All note times are checked before the first synthesis request.
    for item in events:
        frequency = 440 * 2 ** ((item["midi"] - 69) / 12)
        if not 0 < frequency < sr / 2:
            raise ValueError(f"{item['layer']} pitch exceeds the requested Nyquist range; use a higher sample rate")
        start = int(item["start_seconds"] * sr)
        end = start + int(item["duration_seconds"] * sr)
        if start < 0 or end > frames or end <= start:
            raise ValueError("scheduled note falls outside the complete song")
    ledger, counts = [], {role: 0 for role in ROLES}
    for item in events:
        role = item["layer"]
        voice = song["voices"][role]
        frequency = 440 * 2 ** ((item["midi"] - 69) / 12)
        duration = item["duration_seconds"]
        if voice.startswith("classic:"):
            note = _classic(synth, voice.split(":", 1)[1], frequency, duration, item["velocity"], sr)
        else:
            note = synth.instrument_note(frequency, duration, voice, volume=1., sr=sr,
                                         velocity=item["velocity"], seed=item["id"])
        note = synth._audio_array(note) * LEVEL[role]
        if note.ndim != 1 or len(note) != int(duration * sr) or np.max(np.abs(note)) <= 1e-8:
            raise ValueError(f"{role} event {item['id']} did not produce its complete nonzero note")
        start = int(item["start_seconds"] * sr)
        layers[role][start:start + len(note)] += note
        counts[role] += 1
        ledger.append({**item, "voice": voice, "mixed": True})
    mixed = np.zeros((frames, 2))
    layer_stats = []
    for role, layer in layers.items():
        angle = (PAN[role] + 1) * np.pi / 4
        mixed[:, 0] += layer * math.cos(angle)
        mixed[:, 1] += layer * math.sin(angle)
        layer_stats.append(dict(name=role, events=counts[role], rms=float(np.sqrt(np.mean(layer * layer))),
                                peak=float(np.max(np.abs(layer)))))
    mixed = synth.make_seamless_loop(synth._protect_playback_peak(mixed, .90), .015, sr=sr)
    for section in sections:
        a, b = int(section["start_seconds"] * sr), int(section["end_seconds"] * sr)
        section["rms"] = float(np.sqrt(np.mean(mixed[a:b] ** 2)))
        section["active_layers"] = [role for role in ROLES if np.max(np.abs(layers[role][a:b])) > 1e-8]
        if section["rms"] < .0001:
            raise ValueError(f"section {section['name']} contains no usable audio")
    audit = dict(version=1, renderer="render_song", engine="pymusic", complete=True, notes_scheduled=len(events), notes_mixed=len(ledger),
                 layer_count=len(layer_stats), section_count=len(sections),
                 active_layers=sum(row["events"] > 0 and row["rms"] > 1e-8 for row in layer_stats),
                 duration_seconds=frames / sr, bars=sum(section["bars"] for section in sections),
                 sample_rate=sr, frames=frames, pcm_sha256=audio_identity(mixed), layer_stats=layer_stats,
                 sections=sections, events=ledger,
                 title=song["title"], key=f"{song['tonic']} {song['mode']}", bpm=song["bpm"])
    if audit["active_layers"] != 5 or len(ledger) != len(events):
        raise ValueError("the complete song did not place every event in all five parts")
    return mixed, audit

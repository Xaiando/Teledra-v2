"""Read ProTracker-style Amiga MOD modules into note events. No audio is decoded.

A MOD holds 31 sample headers, an order list and patterns of 64 rows, each row
with one cell per channel (sample, Amiga period, effect, parameter). This reader
walks the order list once, following speed and tempo changes and pattern breaks,
and stops at a backward position jump so a looping song is read once. Periods map
to the ProTracker note table with C-2 as MIDI 60. Timing assumes four rows per
beat, the common tracker convention, so a beat is ``speed * 4`` ticks.
"""
from __future__ import annotations

SIGNATURES = {"M.K.": 4, "M!K!": 4, "FLT4": 4, "4CHN": 4, "6CHN": 6, "8CHN": 8, "FLT8": 8}
# ProTracker period table at finetune 0, C-1 to B-3.
PERIODS = (856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453,
           428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226,
           214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113)
DRUM_WORDS = ("kick", "bd", "bass drum", "bassdrum", "snare", "sn", "hat", "hh", "hihat", "drum", "perc", "tom", "clap", "cymbal", "crash", "ride")


def channel_count(data):
    if len(data) < 1084:
        return None
    signature = data[1080:1084].decode("latin-1")
    if signature in SIGNATURES:
        return SIGNATURES[signature]
    if signature[2:] == "CH" and signature[:2].isdigit():
        return int(signature[:2])
    return None


def period_to_midi(period):
    index = min(range(len(PERIODS)), key=lambda i: abs(PERIODS[i] - period))
    return 48 + index


def read_mod(data):
    """Return {"title", "channels", "samples", "notes"}; each note has channel, beat, beats, midi, sample, volume, arpeggio."""
    channels = channel_count(data)
    if channels is None:
        raise ValueError("Not a 31-sample ProTracker module")
    samples = []
    for index in range(31):
        base = 20 + index * 30
        samples.append({"number": index + 1, "name": data[base:base + 22].split(b"\0")[0].decode("latin-1", "replace").strip(),
                        "length": int.from_bytes(data[base + 22:base + 24], "big") * 2, "volume": data[base + 25]})
    song_length = data[950]
    orders = list(data[952:952 + 128])
    pattern_bytes = 64 * channels * 4
    speed, tempo, row_count = 6, 125, 0
    opening = None
    ticks = 0.
    cells = []
    order_index, start_row, visited = 0, 0, set()
    while order_index < song_length:
        if (order_index, start_row) in visited:
            break
        visited.add((order_index, start_row))
        base = 1084 + orders[order_index] * pattern_bytes
        if base + pattern_bytes > len(data):
            break
        next_order, next_row = order_index + 1, 0
        for row in range(start_row, 64):
            jump = None
            for channel in range(channels):
                offset = base + (row * channels + channel) * 4
                b0, b1, b2, b3 = data[offset:offset + 4]
                sample = (b0 & 0xF0) | (b2 >> 4)
                period = ((b0 & 0x0F) << 8) | b1
                effect, parameter = b2 & 0x0F, b3
                if effect == 0xF and parameter:
                    if parameter < 32:
                        speed = parameter
                    else:
                        tempo = parameter
                if period:
                    if opening is None:
                        opening = (speed, tempo)
                    cells.append({"channel": channel, "tick": ticks, "speed": speed, "midi": period_to_midi(period),
                                  "sample": sample, "volume": parameter if effect == 0xC else None,
                                  "arpeggio": [parameter >> 4, parameter & 0x0F] if effect == 0 and parameter else None})
                if effect == 0xD:
                    jump = (order_index + 1, min(63, (parameter >> 4) * 10 + (parameter & 0x0F)))
                elif effect == 0xB:
                    jump = (parameter, 0) if parameter > order_index else (song_length, 0)
            ticks += speed
            row_count += 1
            if jump is not None:
                next_order, next_row = jump
                break
        order_index, start_row = next_order, next_row
    notes = []
    last_sample = {}
    by_channel = {}
    for cell in cells:
        if cell["sample"]:
            last_sample[cell["channel"]] = cell["sample"]
        cell["sample"] = cell["sample"] or last_sample.get(cell["channel"], 0)
        by_channel.setdefault(cell["channel"], []).append(cell)
    for channel, line in by_channel.items():
        for current, following in zip(line, line[1:] + [None]):
            ticks_per_beat = current["speed"] * 4
            end = following["tick"] if following else current["tick"] + ticks_per_beat
            notes.append({"channel": channel, "beat": current["tick"] / ticks_per_beat,
                          "beats": max(.25 / 4, min(4., (end - current["tick"]) / ticks_per_beat)),
                          "midi": current["midi"], "sample": current["sample"], "volume": current["volume"],
                          "arpeggio": current["arpeggio"]})
    notes.sort(key=lambda n: (n["beat"], n["channel"]))
    opening_speed, opening_tempo = opening or (speed, tempo)
    return {"title": data[:20].split(b"\0")[0].decode("latin-1", "replace").strip(), "channels": channels,
            "samples": samples, "rows": row_count, "notes": notes,
            # Four rows per beat: beats per minute = tempo * 6 / speed.
            "bpm": int(round(opening_tempo * 6 / opening_speed))}


def is_drum_sample(sample):
    name = sample["name"].lower()
    return any(word in name for word in DRUM_WORDS)

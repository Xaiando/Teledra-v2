"""Console-era voices for Buzzart: NES-style pulse, triangle and noise, simple FM, and Amiga-style samples.

These imitate the character of classic sound hardware with plain synthesis:
stepped 4-bit volume and triangle, narrow pulse duty cycles, delayed vibrato,
held-sample noise drums, two-operator FM in the manner of the Sega Genesis
chip, and 8-bit sample loops played without interpolation through a fixed
low-pass in the manner of the Amiga's Paula chip. Every waveform and drum
sample is synthesized here; they contain no recorded samples and no game music.
``note_factory`` returns a renderer hook that produces exactly the frames the
Fly renderer checks.
"""
from __future__ import annotations

import hashlib
import json

import numpy as np

CHIP_VOICES = {
    "chip:pulse12": "NES-style pulse at 12.5% duty: thin, nasal lead",
    "chip:pulse25": "NES-style pulse at 25% duty: bright lead",
    "chip:pulse50": "NES-style square: hollow lead or chord tones",
    "chip:triangle": "NES-style 4-bit triangle: bass",
    "chip:noise": "NES-style noise: kick below C3, snare to B4, hat above",
    "fm:bass": "Two-operator FM bass in the Sega Genesis manner",
    "fm:lead": "Two-operator FM lead with a bright attack",
    "fm:bell": "Two-operator FM bell",
    "amiga:lead": "Amiga-style looped 8-bit sawtooth sample: bright lead",
    "amiga:pluck": "Amiga-style looped 8-bit square sample with a plucked decay",
    "amiga:chord": "Amiga-style looped 8-bit organ sample for arpeggios",
    "amiga:bass": "Amiga-style looped 8-bit square-and-saw bass",
    "amiga:drums": "Amiga-style 8-bit drum samples at tracker rates: kick below C3, snare to B4, hat above",
}
PALETTES = {
    "nes": {"lead": "chip:pulse25", "answer": "chip:pulse12", "harmony": "chip:pulse50", "bass": "chip:triangle", "texture": "chip:noise"},
    "genesis": {"lead": "fm:lead", "answer": "chip:pulse25", "harmony": "fm:bell", "bass": "fm:bass", "texture": "chip:noise"},
    "amiga": {"lead": "amiga:lead", "answer": "amiga:pluck", "harmony": "amiga:chord", "bass": "amiga:bass", "texture": "amiga:drums"},
    # Retro on the top and bottom, PyMusic in the middle. A sound chip cannot
    # hold a lush chord, which is why chiptune arpeggiates, so the sustained
    # parts go to PyMusic patches that can: a warm pad under the melody and an
    # electric piano answering it. The FM lead and bass keep the Sonic colour,
    # and the drums stay on the chip.
    "indie": {"lead": "fm:lead", "answer": "electric_piano", "harmony": "warm_pad", "bass": "fm:bass", "texture": "chip:noise"},
    # Three styles in the indie territory, pulling in different directions.
    "dusk": {"lead": "electric_piano", "answer": "vibraphone", "harmony": "warm_pad", "bass": "fm:bass", "texture": "chip:noise"},
    "overworld": {"lead": "fm:lead", "answer": "marimba", "harmony": "electric_guitar", "bass": "fm:bass", "texture": "chip:noise"},
    "dreamy": {"lead": "chip:triangle", "answer": "celesta", "harmony": "choir", "bass": "double_bass", "texture": "glockenspiel"},
    # A stress test: every part busy, sevenths, strings, heavy drums.
    "complex": {"lead": "fm:lead", "answer": "vibraphone", "harmony": "string_ensemble", "bass": "fm:bass", "texture": "chip:noise"},
}
LOOP_FRAMES = 32  # frames in one cycle of a synthesized Amiga-style loop sample
# A short moving average at 48 kHz stands in for the Amiga 500's fixed low-pass, near 4.5 kHz.
A500_SMOOTHING = 5


def _envelope(frames, sr, attack=.004, decay=.12, sustain=.7, release=.03, steps=None):
    t = np.arange(frames) / sr
    rise = np.minimum(1., t / attack) if attack > 0 else np.ones(frames)
    fall = sustain + (1 - sustain) * np.exp(-np.maximum(0., t - attack) / max(decay, 1e-4))
    envelope = np.where(t < attack, rise, fall)
    tail = min(frames, max(1, int(release * sr)))
    envelope[-tail:] *= np.linspace(1., 0., tail)
    if steps:
        # Hardware volume registers move in steps; keep a floor so a quantized tail is never all zero.
        envelope = np.maximum(np.round(envelope * steps) / steps, 1. / (steps * 4))
    return envelope


def _phase(frequency, frames, sr, vibrato=0., delay=.18, rate=5.5):
    t = np.arange(frames) / sr
    wobble = 1. + vibrato * np.sin(2 * np.pi * rate * t) * np.clip((t - delay) / .15, 0., 1.)
    return np.cumsum(frequency * wobble / sr)


def pulse(frequency, frames, sr, duty):
    wave = np.where((_phase(frequency, frames, sr, vibrato=.006) % 1.) < duty, 1., -1.)
    return wave * _envelope(frames, sr, attack=.002, decay=.25, sustain=.55, steps=15)


def triangle(frequency, frames, sr):
    ramp = _phase(frequency, frames, sr) % 1.
    wave = 1. - 4. * np.abs(ramp - .5)
    wave = np.round((wave + 1.) * 7.5) / 7.5 - 1.  # 16 output levels, like the NES triangle channel
    return wave * _envelope(frames, sr, attack=.002, decay=1., sustain=1., release=.02)


def noise(midi, frames, sr, seed):
    rng = np.random.default_rng(seed)
    t = np.arange(frames) / sr
    if midi < 48:
        hold, decay = 40, .09
    elif midi < 72:
        hold, decay = 6, .13
    else:
        hold, decay = 1, .045
    wave = np.repeat(rng.choice((-1., 1.), size=frames // hold + 1), hold)[:frames]
    if midi < 48:
        sweep = np.cumsum(np.linspace(130., 45., frames) / sr)
        wave = .45 * wave + .9 * np.sin(2 * np.pi * sweep)
    return wave * np.exp(-t / decay)


def fm(frequency, frames, sr, ratio, index, decay, sustain):
    t = np.arange(frames) / sr
    depth = index * (.25 + .75 * np.exp(-t / decay))
    modulator = np.sin(2 * np.pi * frequency * ratio * t) * depth
    return np.sin(2 * np.pi * frequency * t + modulator) * _envelope(frames, sr, attack=.003, decay=decay, sustain=sustain)


def _eight_bit(wave):
    peak = float(np.max(np.abs(wave))) or 1.
    return np.round(wave / peak * 127.) / 127.


def _smooth(wave, sr):
    width = max(1, int(round(A500_SMOOTHING * sr / 48000)))
    return np.convolve(wave, np.ones(width) / width, mode="same") if 1 < width <= len(wave) else wave


def _loop_table(kind):
    phase = np.arange(LOOP_FRAMES) / LOOP_FRAMES
    square, saw = np.where(phase < .5, 1., -1.), 1. - 2. * phase
    wave = {"saw": saw, "square": square, "bass": .6 * square + .4 * saw,
            "organ": np.sin(2 * np.pi * phase) + .5 * np.sin(4 * np.pi * phase) + .33 * np.sin(8 * np.pi * phase)}[kind]
    return _eight_bit(wave)


def paula_loop(kind, frequency, frames, sr, vibrato=0., decay=.2, sustain=.8):
    """A looped 8-bit sample stepped through without interpolation, with 64-step volume."""
    table = _loop_table(kind)
    position = np.floor(_phase(frequency, frames, sr, vibrato=vibrato) * LOOP_FRAMES).astype(np.int64)
    wave = _smooth(table[position % LOOP_FRAMES], sr)
    return wave * _envelope(frames, sr, attack=.003, decay=decay, sustain=sustain, steps=64)


def paula_drum(midi, frames, sr, seed):
    """An 8-bit drum sample made at a tracker sample rate, played back held between its frames."""
    rng = np.random.default_rng(seed)
    if midi < 48:
        rate, length = 8363, .28
        t = np.arange(int(rate * length)) / rate
        sample = np.sin(2 * np.pi * np.cumsum(np.linspace(160., 40., len(t)) / rate)) * np.exp(-t / .11)
    elif midi < 72:
        rate, length = 16726, .22
        t = np.arange(int(rate * length)) / rate
        sample = (.7 * rng.uniform(-1., 1., len(t)) + .5 * np.sin(2 * np.pi * 185. * t)) * np.exp(-t / .07)
    else:
        rate, length = 22000, .09
        t = np.arange(int(rate * length)) / rate
        sample = rng.uniform(-1., 1., len(t)) * np.exp(-t / .025)
    sample = _eight_bit(sample)
    position = np.floor(np.arange(frames) * rate / sr).astype(np.int64)
    wave = np.where(position < len(sample), sample[np.minimum(position, len(sample) - 1)], 0.)
    return _smooth(wave, sr)


def render_voice(voice, event, sr, seed=0):
    frames = int(event["duration_seconds"] * sr)
    frequency = 440. * 2 ** ((event["midi"] - 69) / 12)
    if voice == "chip:pulse12":
        sound = pulse(frequency, frames, sr, .125)
    elif voice == "chip:pulse25":
        sound = pulse(frequency, frames, sr, .25)
    elif voice == "chip:pulse50":
        sound = pulse(frequency, frames, sr, .5)
    elif voice == "chip:triangle":
        sound = triangle(frequency, frames, sr)
    elif voice == "chip:noise":
        identity = json.dumps([seed, event["layer"], event["start_beat"], event["midi"]])
        sound = noise(event["midi"], frames, sr, int.from_bytes(hashlib.sha256(identity.encode()).digest()[:4], "little"))
    elif voice == "fm:bass":
        sound = fm(frequency, frames, sr, ratio=1., index=2.2, decay=.09, sustain=.55)
    elif voice == "fm:lead":
        sound = fm(frequency, frames, sr, ratio=2., index=1.6, decay=.18, sustain=.6)
    elif voice == "fm:bell":
        sound = fm(frequency, frames, sr, ratio=3.5, index=2.8, decay=.35, sustain=.15)
    elif voice == "amiga:lead":
        sound = paula_loop("saw", frequency, frames, sr, vibrato=.005, decay=.3, sustain=.75)
    elif voice == "amiga:pluck":
        sound = paula_loop("square", frequency, frames, sr, decay=.09, sustain=.2)
    elif voice == "amiga:chord":
        sound = paula_loop("organ", frequency, frames, sr, decay=.12, sustain=.5)
    elif voice == "amiga:bass":
        sound = paula_loop("bass", frequency, frames, sr, decay=.25, sustain=.7)
    elif voice == "amiga:drums":
        # A tracker replays one sample per drum, so every hit of a kind shares its sample within a piece.
        kind = 0 if event["midi"] < 48 else 1 if event["midi"] < 72 else 2
        identity = json.dumps([seed, "amiga-drum", kind])
        sound = paula_drum(event["midi"], frames, sr, int.from_bytes(hashlib.sha256(identity.encode()).digest()[:4], "little"))
    else:
        raise ValueError(f"Unknown console voice: {voice}")
    return np.asarray(sound, dtype=float)


def is_console_palette(voices):
    return all(voice in CHIP_VOICES for voice in voices.values())


def uses_chip_voices(voices):
    """True when any part needs the chip synth, so rendering must go through the chain."""
    return any(voice in CHIP_VOICES for voice in voices.values())


def pymusic_voices(voices):
    """The parts that fall through the chain to PyMusic."""
    return {role: voice for role, voice in voices.items() if voice not in CHIP_VOICES}


def note_factory(voices, seed=0, score=None):
    """A renderer hook that chains the two synths, one note at a time.

    Palettes used to be all or nothing: every voice a chip voice, or none. That
    kept the sustained PyMusic patches — pads, strings, electric piano — out of
    every console-style piece, and those are exactly the voices slow music needs
    for layers. Each note now goes to the chip if its part names a chip voice,
    and otherwise on to PyMusic with the same call and the same per-note seed the
    ordinary Fly renderer uses, so a PyMusic part sounds identical either way.
    The PyMusic bridge must already be open; the caller owns its lifetime.
    """
    from ..render import native_duration, note_seed
    from ..vendor import teledra_synth as synth

    def render(event, sr):
        voice = voices[event["layer"]]
        if voice in CHIP_VOICES:
            return render_voice(voice, event, sr, seed)
        frequency = 440. * 2 ** ((event["midi"] - 69) / 12)
        return synth.instrument_note(frequency, native_duration(event["duration_seconds"], sr), voice, volume=1., sr=sr,
                                     velocity=event["velocity"],
                                     # The renderer hands over the validation stand-in, whose voice is
                                     # classic:sine; the seed must hash the real voice or a PyMusic part
                                     # would get different timbre noise here than on the ordinary path.
                                     seed=note_seed(score, {**event, "voice": voice}) if score is not None else seed)
    return render

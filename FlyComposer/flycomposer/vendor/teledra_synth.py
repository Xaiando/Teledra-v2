"""V1-compatible, device-free NumPy music authoring plus the native instrument catalog.

No import, helper or play_sound call opens an audio device. The render worker
captures one finished array; Teledra owns the only playback stream.
"""
from __future__ import annotations

import atexit
from collections import OrderedDict
from collections.abc import Sequence
import json
import math
import re
import subprocess
import threading

import numpy as np

PLAYBACK_PEAK_CEILING = 0.95
MAX_SECONDS = 600.0
MAX_SAMPLES = 48_000 * 600 * 2
_bridge = None
_capture_callback = None
_note_requests = 0
_note_renders = 0
_play_requests = 0
_song_requests = 0
_song_renders = 0
_song_audit = None
_note_cache = OrderedDict()
_cache_bytes = 0


def _audio_array(wave):
    array = np.asarray(wave, dtype=np.float64)
    if array.ndim == 0:
        array = array.reshape(1)
    elif array.ndim == 2 and array.shape[1] not in (1, 2) and array.shape[0] in (1, 2):
        array = array.T
    if array.ndim not in (1, 2) or (array.ndim == 2 and array.shape[1] not in (1, 2)):
        raise ValueError("audio must be mono or one/two-channel stereo")
    if array.size > MAX_SAMPLES:
        raise ValueError("audio exceeds the bounded render size")
    if not np.all(np.isfinite(array)):
        raise ValueError("audio contains a non-finite sample")
    return array


def _duration_frames(duration, sr):
    duration, sr = float(duration), int(sr)
    if not math.isfinite(duration) or not 0 < duration <= MAX_SECONDS:
        raise ValueError("duration must be finite and in (0, 600] seconds")
    if not 1000 <= sr <= 192000:
        raise ValueError("sample rate must be 1000..192000 Hz")
    frames = int(duration * sr)
    if frames < 2 or frames > MAX_SAMPLES:
        raise ValueError("note duration must contain at least two bounded samples")
    return frames


def note_to_freq(note):
    """Convert C4/Eb3/A#2 to Hz; numeric input already means Hz, as in V1."""
    if note is None or (isinstance(note, str) and note.strip().lower() in ("", "rest", "r")):
        return 0.0
    if not isinstance(note, str):
        value = float(note)
        if not math.isfinite(value) or value < 0:
            raise ValueError("frequency must be finite and nonnegative")
        return value
    match = re.fullmatch(r"([a-gA-G])([#bB]?)(-?\d+)?", note.strip())
    if match is None:
        raise ValueError(f"invalid note name: {note!r}")
    letter, accidental, octave = match.groups()
    pitch = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}[letter.upper()]
    pitch += 1 if accidental == "#" else -1 if accidental else 0
    midi = (int(octave) if octave is not None else 4) * 12 + 12 + pitch
    return 440.0 * 2.0 ** ((midi - 69) / 12)


def generate_wave(freq, duration, wave_type="sine", sr=44100, duty=0.5, **kwargs):
    frames = _duration_frames(duration, sr)
    freq = note_to_freq(freq)
    if freq == 0:
        return np.zeros(frames)
    if freq >= sr / 2:
        raise ValueError("frequency must be below the requested sample rate's Nyquist limit")
    t = np.arange(frames, dtype=float) / sr
    kind = wave_type.lower().replace("-", "_").replace(" ", "_")
    if kind == "sine":
        return np.sin(2 * np.pi * freq * t)
    if kind in ("saw", "sawtooth"):
        return 2 * (t * freq - np.floor(0.5 + t * freq))
    if kind in ("square", "pulse"):
        if not 0 < float(duty) < 1:
            raise ValueError("duty must lie between zero and one")
        return np.where((t * freq) % 1 < duty, 1.0, -1.0)
    if kind == "triangle":
        return 2 * np.abs(2 * (t * freq - np.floor(t * freq + 0.5))) - 1
    if kind in ("pink", "pink_noise"):
        return pink_noise(duration, sr=sr, seed=kwargs.get("seed"), rng=kwargs.get("rng"))
    if kind in ("noise", "white", "white_noise"):
        return _noise_rng(seed=kwargs.get("seed"), rng=kwargs.get("rng")).uniform(-1, 1, frames)
    raise ValueError(f"unknown waveform {wave_type!r}; use instrument_note for named instruments")


def synth_note(note, duration, wave_type="sine", attack=0.05, decay=0.1,
               sustain=0.7, release=0.2, volume=0.2, sr=44100, **kwargs):
    """V1 exact-duration waveform note; overlays and rhythm remain authored Python."""
    global _note_requests, _note_renders
    _note_requests += 1
    _duration_frames(duration, sr)
    parameters = [attack, decay, sustain, release, volume]
    if not all(math.isfinite(float(value)) for value in parameters):
        raise ValueError("note envelope and volume must be finite")
    if min(attack, decay, release, volume) < 0 or not 0 <= sustain <= 1:
        raise ValueError("envelope times/volume must be nonnegative; sustain must be 0..1")
    frequency = note_to_freq(note)
    wave = generate_wave(frequency, duration, wave_type, sr, **kwargs)
    result = _audio_array(wave * adsr_envelope(duration, attack, decay, sustain, release, sr) * volume)
    _note_renders += 1
    return result


class _NativeBridge:
    def __init__(self, executable):
        flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
        self.process = subprocess.Popen([str(executable), "--music-note-server"],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, creationflags=flags)
        self.lock = threading.Lock()
        self.catalog = None

    def close(self):
        if self.process.poll() is None:
            try:
                self.process.stdin.close()
                self.process.wait(timeout=2)
            except (OSError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait(timeout=2)
        if not self.process.stdin.closed:
            self.process.stdin.close()
        self.process.stdout.close()

    def request(self, payload):
        with self.lock:
            if self.process.poll() is not None:
                raise RuntimeError("native note renderer exited before completing the request")
            self.process.stdin.write(json.dumps(payload, allow_nan=False).encode("utf-8") + b"\n")
            self.process.stdin.flush()
            line = self.process.stdout.readline(65537)
            if not line.endswith(b"\n") or len(line) > 65536:
                raise RuntimeError("native note renderer returned an incomplete response header")
            header = json.loads(line)
            if not header.get("ok"):
                raise ValueError(header.get("error", "native note request failed"))
            if payload["kind"] == "catalog":
                catalog = header.get("instruments")
                if not isinstance(catalog, list) or len(catalog) < 70:
                    raise RuntimeError("native instrument catalog is incomplete")
                return catalog
            frames = header.get("frames")
            if (header.get("sample_rate") != 48000 or header.get("channels") != 1
                    or not isinstance(frames, int) or not 1 <= frames <= 48000 * 30
                    or header.get("bytes") != frames * 4):
                raise RuntimeError("native note renderer returned an invalid PCM header")
            chunks, remaining = [], frames * 4
            while remaining:
                chunk = self.process.stdout.read(remaining)
                if not chunk:
                    raise RuntimeError("native note renderer returned truncated PCM")
                chunks.append(chunk)
                remaining -= len(chunk)
            return _audio_array(np.frombuffer(b"".join(chunks), dtype="<f4").astype(float))


def _configure_bridge(executable):
    global _bridge
    _close_bridge()
    _bridge = _NativeBridge(executable) if executable else None


def _close_bridge():
    global _bridge, _cache_bytes
    if _bridge is not None:
        _bridge.close()
        _bridge = None
    _note_cache.clear()
    _cache_bytes = 0


def instruments():
    """Return all executable native instruments and their range/role metadata."""
    if _bridge is None:
        raise RuntimeError("named instruments require the Teledra native note renderer")
    if _bridge.catalog is None:
        _bridge.catalog = _bridge.request({"kind": "catalog"})
    return [dict(item) for item in _bridge.catalog]


class _InstrumentIds(Sequence):
    def __len__(self):
        return len(instruments())

    def __getitem__(self, index):
        return tuple(item["id"] for item in instruments())[index]


INSTRUMENTS = _InstrumentIds()


def instrument_note(note, duration, instrument="piano", volume=0.2, sr=44100, **kwargs):
    """Render one exact-duration native patch, with no arranger or omitted attacks."""
    global _note_requests, _note_renders, _cache_bytes
    _note_requests += 1
    frames = _duration_frames(duration, sr)
    if not 0.001 <= float(duration) <= 30:
        raise ValueError("native instrument note duration must be .001..30 seconds")
    frequency = note_to_freq(note)
    if not math.isfinite(float(volume)) or volume < 0:
        raise ValueError("volume must be finite and nonnegative")
    if frequency == 0:
        _note_renders += 1
        return np.zeros(frames)
    if frequency >= sr / 2:
        raise ValueError("frequency must be below the requested sample rate's Nyquist limit")
    if instrument not in INSTRUMENTS:
        raise ValueError(f"unsupported instrument ID: {instrument!r}")
    velocity = float(kwargs.get("velocity", 1.0))
    seed = int(kwargs.get("seed", 0))
    if not 0 <= velocity <= 1 or not 0 <= seed <= 0xffffffff:
        raise ValueError("velocity must be 0..1 and seed must be a u32")
    payload = dict(kind="note", instrument=instrument, frequency=frequency,
                   duration=float(duration), velocity=velocity, seed=seed)
    if "glide_from" in kwargs:
        payload["glide_from_frequency"] = note_to_freq(kwargs["glide_from"])
        payload["glide_seconds"] = float(kwargs.get("glide_seconds", 0.12))
    key = json.dumps(payload, sort_keys=True, allow_nan=False)
    if key in _note_cache:
        raw = _note_cache[key]
        _note_cache.move_to_end(key)
    else:
        raw = _bridge.request(payload)
        expected = int(float(duration) * 48000)
        # The server parses the duration from JSON and may land one frame
        # either side of Python's exact product (45999 for 46000). A frame of
        # difference is rounding, not a lost note: the endpoint sample is
        # silent, so pad or trim it; anything larger is still refused.
        if abs(len(raw) - expected) == 1:
            raw = np.concatenate([raw, np.zeros(1, dtype=raw.dtype)]) if len(raw) < expected else raw[:expected]
        if len(raw) != expected:
            raise RuntimeError(f"native note was incomplete: {len(raw)} frames, expected {expected}")
        while _note_cache and _cache_bytes + raw.nbytes > 64 * 1024 * 1024:
            _, removed = _note_cache.popitem(last=False)
            _cache_bytes -= removed.nbytes
        _note_cache[key] = raw
        _cache_bytes += raw.nbytes
    # Native synthesis is48k; V1 authors at44.1k by default. Resample the full
    # note without skipping events or changing its authored duration.
    if int(sr) == 48000 and frames == len(raw):
        result = raw.copy()
    else:
        source = raw
        if sr < 48000:
            spectrum = np.fft.rfft(raw)
            spectrum[np.fft.rfftfreq(len(raw), 1 / 48000) >= sr / 2] = 0
            source = np.fft.irfft(spectrum, n=len(raw))
        result = np.interp(np.arange(frames) * 48000.0 / sr, np.arange(len(source)), source)
    _note_renders += 1
    return result * volume


def render_song(song, sr=44100):
    """Render one validated complete five-part plan; never open an audio device.

    Classic recipes reuse V1 waveforms and envelopes; exact native IDs use the
    full instrument catalog. The worker binds the private placement audit to
    the actual final array, rather than trusting authored metadata.
    """
    global _song_requests, _song_renders, _song_audit
    import sys
    from pymusic_score import render
    _song_requests += 1
    result, audit = render(song, sr, sys.modules[__name__])
    _song_renders += 1
    _song_audit = audit
    return result


def play_sound(wave, sr=44100, loop=False, **kwargs):
    """Submit exactly one completed array to the worker; never open audio/UI."""
    global _play_requests
    _play_requests += 1
    if _capture_callback is None:
        raise RuntimeError("play_sound must run inside Teledra's Python music worker")
    return _capture_callback(_protect_playback_peak(wave), int(sr), bool(loop))


atexit.register(_close_bridge)

def stereo_pan(wave, pan=0.0):
    """Place mono/stereo audio with equal-power pan: -1 left, 0 center, 1 right."""
    array = _audio_array(wave)
    mono = array if array.ndim == 1 else np.mean(array, axis=1)
    pan = float(np.clip(pan, -1.0, 1.0))
    angle = (pan + 1.0) * np.pi / 4.0
    return np.column_stack((mono * np.cos(angle), mono * np.sin(angle)))

def stereo_width(wave, width=1.0):
    """Adjust stereo side energy while preserving the center image."""
    array = _audio_array(wave)
    if array.ndim == 1 or array.shape[1] == 1:
        array = stereo_pan(array.reshape(-1), 0.0)
    width = float(np.clip(width, 0.0, 2.0))
    mid = (array[:, 0] + array[:, 1]) * 0.5
    side = (array[:, 0] - array[:, 1]) * 0.5 * width
    return np.column_stack((mid + side, mid - side))

def automation_curve(duration, points, sr=44100):
    """Interpolate `(time_seconds, value)` points into a sample-accurate curve."""
    samples = max(1, int(float(duration) * int(sr)))
    clean = []
    for time_seconds, value in points:
        clean.append((float(np.clip(time_seconds, 0.0, duration)), float(value)))
    if not clean:
        return np.ones(samples)
    clean.sort(key=lambda item: item[0])
    if clean[0][0] > 0.0:
        clean.insert(0, (0.0, clean[0][1]))
    if clean[-1][0] < duration:
        clean.append((float(duration), clean[-1][1]))
    times = np.asarray([item[0] for item in clean], dtype=float)
    values = np.asarray([item[1] for item in clean], dtype=float)
    timeline = np.arange(samples, dtype=float) / float(sr)
    return np.interp(timeline, times, values)

def apply_automation(wave, curve):
    """Apply a one-dimensional control curve to mono or stereo audio."""
    array = _audio_array(wave)
    control = fit_to_length(np.asarray(curve, dtype=float).reshape(-1), len(array), mode="pad")
    return array * control if array.ndim == 1 else array * control[:, None]

def soft_limiter(wave, drive=1.2, ceiling=0.92):
    """Softly bend peaks toward a ceiling without normalizing a weak mix upward."""
    array = _audio_array(wave).copy()
    ceiling = float(np.clip(ceiling, 0.1, 0.99))
    drive = max(1.0, float(drive))
    threshold = ceiling / drive
    magnitude = np.abs(array)
    above = magnitude > threshold
    if np.any(above):
        span = max(1e-9, ceiling - threshold)
        compressed = threshold + span * np.tanh((magnitude[above] - threshold) / span)
        array[above] = np.sign(array[above]) * compressed
    return np.clip(array, -ceiling, ceiling)

def adsr_envelope(duration, attack=0.05, decay=0.1, sustain=0.7, release=0.2, sr=44100):
    """
    Generates an ADSR envelope of a given duration.
    """
    total_samples = int(duration * sr)
    a_samples = int(attack * sr)
    d_samples = int(decay * sr)
    r_samples = int(release * sr)

    # Bound segments
    if a_samples + d_samples + r_samples > total_samples:
        # scale down proportionally
        scale = total_samples / (a_samples + d_samples + r_samples + 1)
        a_samples = int(a_samples * scale)
        d_samples = int(d_samples * scale)
        r_samples = int(r_samples * scale)

    s_samples = total_samples - (a_samples + d_samples + r_samples)

    env = np.zeros(total_samples)

    # Attack
    if a_samples > 0:
        env[:a_samples] = np.linspace(0.0, 1.0, a_samples)

    # Decay
    if d_samples > 0:
        env[a_samples:a_samples+d_samples] = np.linspace(1.0, sustain, d_samples)

    # Sustain
    if s_samples > 0:
        env[a_samples+d_samples:a_samples+d_samples+s_samples] = sustain

    # Release
    if r_samples > 0:
        env[-r_samples:] = np.linspace(sustain, 0.0, r_samples)

    return env

def _noise_rng(seed=None, rng=None):
    """Use a supplied generator, a repeatable seed, or NumPy's legacy global RNG."""
    if rng is not None:
        return rng
    if seed is not None:
        return np.random.default_rng(seed)
    return np.random

def pink_noise(duration=1.0, volume=1.0, sr=44100, seed=None, rng=None, **kwargs):
    """Generate bounded pink noise whose power falls by roughly 3 dB per octave.

    ``seed`` and ``rng`` are optional so existing calls stay nondeterministic while
    tests and repeatable compositions can request the exact same texture.
    """
    samples = max(0, int(float(duration) * int(sr)))
    if samples == 0:
        return np.zeros(0, dtype=float)

    generator = _noise_rng(seed=seed, rng=rng)
    white = generator.standard_normal(samples)
    spectrum = np.fft.rfft(white)
    bins = np.arange(len(spectrum), dtype=float)
    spectrum[0] = 0.0
    if len(spectrum) > 1:
        spectrum[1:] /= np.sqrt(bins[1:])
    colored = np.fft.irfft(spectrum, n=samples)

    peak = float(np.max(np.abs(colored)))
    if peak > 0.0:
        colored = colored / peak
    return colored * float(volume)

def delay(wave, delay_time=0.25, feedback=0.4, mix=0.3, sr=44100, **kwargs):
    """
    Applies a simple delay effect to the audio wave.
    """
    # Fallback mappings for common parameter naming variations from LLM
    feedback = kwargs.get('feedback_gain', feedback)
    mix = kwargs.get('mix_gain', mix)

    wave = _audio_array(wave)
    d_samples = int(delay_time * sr)
    if d_samples <= 0 or d_samples >= len(wave):
        return wave

    out = np.copy(wave)
    # Loop feedback taps
    for i in range(1, 4):
        shift = d_samples * i
        if shift >= len(wave):
            break
        out[shift:] += wave[:-shift] * (feedback ** i)

    return (1.0 - mix) * wave + mix * out

def lowpass_filter(wave, cutoff=1000.0, sr=44100, **kwargs):
    """
    Applies a simple moving average lowpass filter to warm up the tone.
    Uses an O(N) cumulative-sum boxcar instead of np.convolve (O(N*k)),
    which was extremely slow for long tracks with low cutoffs.
    """
    N = max(1, int(sr / max(float(cutoff), 1.0)))
    array = _audio_array(wave)
    if N == 1 or len(array) == 0:
        return array

    def filter_channel(channel):
        cs = np.cumsum(np.concatenate(([0.0], channel)))
        idx = np.arange(len(channel))
        left = N // 2
        right = N - left
        hi = np.clip(idx + right, 0, len(channel))
        lo = np.clip(idx - left, 0, len(channel))
        return (cs[hi] - cs[lo]) / N

    if array.ndim == 1:
        return filter_channel(array)
    return np.column_stack([filter_channel(array[:, channel]) for channel in range(array.shape[1])])

def reverb(wave, room_size=0.7, mix=0.2, sr=44100, **kwargs):
    """
    Applies a simple comb-filter-like ambient reverb.
    """
    wave = _audio_array(wave)
    out = np.copy(wave)
    # Small prime-like delay offsets for natural reflection density
    delays = [int(0.029 * sr), int(0.037 * sr), int(0.043 * sr), int(0.053 * sr)]
    for d in delays:
        if d < len(wave):
            decay = room_size * 0.5
            ref = np.zeros_like(wave)
            ref[d:] = wave[:-d] * decay
            out += ref
    return (1.0 - mix) * wave + mix * out

def granular_synthesis(wave, grain_size=0.1, overlap=0.5, pitch_shift=1.0, jitter=0.0, sr=44100, **kwargs):
    """
    Rebuilds a wave from short windowed grains for shimmering, fractured textures.
    grain_size and overlap are measured in seconds and ratio respectively.
    """
    wave = _audio_array(wave)
    if wave.ndim == 2:
        rendered = [
            granular_synthesis(
                wave[:, channel],
                grain_size=grain_size,
                overlap=overlap,
                pitch_shift=pitch_shift,
                jitter=jitter,
                sr=sr,
                **kwargs,
            )
            for channel in range(wave.shape[1])
        ]
        target = max(len(channel) for channel in rendered)
        return np.column_stack([fit_to_length(channel, target) for channel in rendered])
    if len(wave) == 0:
        return wave

    grain_samples = max(2, int(float(grain_size) * sr))
    overlap = float(np.clip(overlap, 0.0, 0.95))
    hop_samples = max(1, int(grain_samples * (1.0 - overlap)))
    pitch_shift = max(0.05, float(pitch_shift))
    jitter_samples = max(0, int(abs(float(jitter)) * sr))

    output_len = len(wave) + grain_samples
    out = np.zeros(output_len)
    window = np.hanning(grain_samples)
    if not np.any(window):
        window = np.ones(grain_samples)

    write_pos = 0
    for read_pos in range(0, len(wave), hop_samples):
        source_pos = read_pos
        if jitter_samples:
            source_pos += np.random.randint(-jitter_samples, jitter_samples + 1)
            source_pos = int(np.clip(source_pos, 0, max(0, len(wave) - 1)))

        grain = wave[source_pos:source_pos + grain_samples]
        if len(grain) < grain_samples:
            grain = np.pad(grain, (0, grain_samples - len(grain)))

        if abs(pitch_shift - 1.0) > 0.001:
            pitched_len = max(2, int(grain_samples / pitch_shift))
            src_x = np.linspace(0.0, 1.0, len(grain), endpoint=False)
            pitched_x = np.linspace(0.0, 1.0, pitched_len, endpoint=False)
            grain = np.interp(pitched_x, src_x, grain)
            grain = fit_to_length(grain, grain_samples)

        end_pos = write_pos + grain_samples
        if end_pos > len(out):
            out = np.pad(out, (0, end_pos - len(out)))
        out[write_pos:end_pos] += grain * window
        write_pos += hop_samples

    out = out[:max(len(wave), write_pos)]
    max_val = np.max(np.abs(out)) if len(out) else 0.0
    if max_val > 1.0:
        out = out / max_val
    return out

def fit_to_length(wave, target_length, mode='pad'):
    """
    Returns a 1D wave with exactly target_length samples.
    mode='pad' pads short waves with silence; mode='loop' repeats short waves.
    """
    wave = _audio_array(wave)
    target_length = int(target_length)
    if target_length <= 0:
        shape = (0,) if wave.ndim == 1 else (0, wave.shape[1])
        return np.zeros(shape)
    if len(wave) == target_length:
        return wave
    if len(wave) == 0:
        shape = (target_length,) if wave.ndim == 1 else (target_length, wave.shape[1])
        return np.zeros(shape)
    if len(wave) > target_length:
        return wave[:target_length]
    if mode == 'loop':
        reps = int(np.ceil(target_length / len(wave)))
        if wave.ndim == 1:
            return np.tile(wave, reps)[:target_length]
        return np.tile(wave, (reps, 1))[:target_length]
    shape = (target_length,) if wave.ndim == 1 else (target_length, wave.shape[1])
    out = np.zeros(shape)
    out[:len(wave)] = wave
    return out

def make_seamless_loop(wave, crossfade_seconds=0.05, sr=44100):
    """Ease the tail into the opening sample so loop playback has no hard seam."""
    out = _audio_array(wave).copy()
    if len(out) < 2:
        return out
    samples = min(max(2, int(float(crossfade_seconds) * sr)), len(out) - 1)
    fade = np.linspace(0.0, 1.0, samples, endpoint=True)
    opening_sample = out[0]
    if out.ndim == 1:
        out[-samples:] = out[-samples:] * (1.0 - fade) + opening_sample * fade
    else:
        out[-samples:] = out[-samples:] * (1.0 - fade[:, None]) + opening_sample * fade[:, None]
    return out

def mix_waves(wave_a, wave_b, start_time=0.0, volume_b=1.0, sr=44100, **kwargs):
    """
    Overlay/mix wave_b onto wave_a at a specific start time (in seconds).
    Pads wave_a if wave_b extends past the end of wave_a.
    """
    wave_a = _audio_array(wave_a)
    wave_b = _audio_array(wave_b)
    stereo = wave_a.ndim == 2 or wave_b.ndim == 2
    if stereo:
        if wave_a.ndim == 1:
            wave_a = stereo_pan(wave_a, 0.0)
        if wave_b.ndim == 1:
            wave_b = stereo_pan(wave_b, 0.0)
    start_idx = int(start_time * sr)
    if start_idx < 0:
        wave_b = wave_b[-start_idx:]
        start_idx = 0
    len_a = len(wave_a)
    len_b = len(wave_b)

    required_len = max(len_a, start_idx + len_b)
    shape = (required_len,) if not stereo else (required_len, 2)
    out = np.zeros(shape)
    out[:len_a] = wave_a
    out[start_idx:start_idx+len_b] += wave_b * volume_b
    return out

def _protect_playback_peak(wave, ceiling=PLAYBACK_PEAK_CEILING):
    """Preserve authored level unless uniform attenuation is needed for safety."""
    array = _audio_array(wave)
    if array.size == 0:
        return array
    ceiling = float(ceiling)
    if not 0.0 < ceiling <= 1.0:
        raise ValueError("playback peak ceiling must be in (0, 1]")
    peak = float(np.max(np.abs(array)))
    if not np.isfinite(peak):
        raise ValueError("audio contains a non-finite sample")
    if peak > ceiling:
        return array * (ceiling / peak)
    return array

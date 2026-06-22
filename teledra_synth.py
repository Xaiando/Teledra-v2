import hashlib
import ast
import json
import os
import time
import wave

import numpy as np
import sounddevice as sd


def _apply_audio_device_override():
    """Let the operator pin Teledra's playback to a specific output device / host API.

    The default output here is often an HDMI/NVIDIA endpoint, which on Windows is
    frequently *single-client*: while Teledra loops a track it holds that endpoint,
    so VLC (especially on WASAPI-exclusive) can't open it and "won't launch
    properly." Routing Teledra to a different device -- or to a shared-mixing host
    API like DirectSound -- lets both play at once.

    Controlled entirely by env vars, so default behaviour is unchanged:
      TELEDRA_AUDIO_DEVICE   = output device index, or a case-insensitive name substring
      TELEDRA_AUDIO_HOSTAPI  = host API name substring, e.g. "DirectSound" / "WASAPI" / "MME"

    Set TELEDRA_AUDIO_DEVICE to a different endpoint than VLC uses (the clean fix for
    HDMI single-client contention), or TELEDRA_AUDIO_HOSTAPI=DirectSound to share nicely.
    Any failure here is swallowed -- audio must never break over a routing preference.
    """
    want_dev = os.environ.get("TELEDRA_AUDIO_DEVICE", "").strip()
    want_api = os.environ.get("TELEDRA_AUDIO_HOSTAPI", "").strip().lower()
    if not want_dev and not want_api:
        return
    try:
        hostapis = sd.query_hostapis()
        devices = sd.query_devices()

        api_idx = None
        if want_api:
            for i, ha in enumerate(hostapis):
                if want_api in ha["name"].lower():
                    api_idx = i
                    break

        chosen = None
        if want_dev.isdigit():
            idx = int(want_dev)
            if 0 <= idx < len(devices) and devices[idx]["max_output_channels"] > 0:
                chosen = idx
        elif want_dev:
            for i, d in enumerate(devices):
                if (d["max_output_channels"] > 0
                        and want_dev.lower() in d["name"].lower()
                        and (api_idx is None or d["hostapi"] == api_idx)):
                    chosen = i
                    break

        if chosen is None and api_idx is not None:
            default_out = hostapis[api_idx].get("default_output_device", -1)
            if default_out is not None and default_out >= 0:
                chosen = default_out

        if chosen is not None:
            sd.default.device = (sd.default.device[0], chosen)
            d = sd.query_devices(chosen)
            print(f"[teledra_synth] audio output -> #{chosen} {d['name']} "
                  f"({hostapis[d['hostapi']]['name']})")
    except Exception as exc:
        print(f"[teledra_synth] audio device override ignored: {exc}")


_apply_audio_device_override()

ROOT = os.path.abspath(os.path.dirname(__file__))
MUSIC_PATH = os.path.join(ROOT, "music.py")
KNOWLEDGE_DIR = os.path.join(ROOT, "knowledge")
MUSIC_FEEDBACK_PATH = os.path.join(KNOWLEDGE_DIR, "music_feedback.jsonl")
MUSIC_PLAYLIST_PATH = os.path.join(KNOWLEDGE_DIR, "music_playlist.jsonl")
ORGANIST_VAULT_PATH = os.path.join(KNOWLEDGE_DIR, "organist_music_vault.md")
MUSIC_KEEPERS_DIR = os.path.join(ROOT, "music_experiments", "keepers")
MUSIC_PLAYLIST_DIR = os.path.join(ROOT, "music_experiments", "playlist")
MUSIC_RENDERS_DIR = os.path.join(ROOT, "music_renders")
MUSIC_RENDER_LEDGER_PATH = os.path.join(KNOWLEDGE_DIR, "music_render_provenance.jsonl")


def _current_music_code():
    try:
        with open(MUSIC_PATH, "r", encoding="utf-8") as handle:
            return handle.read()
    except OSError:
        return ""


def _music_metadata_from_code(code):
    """Extract simple literal metadata from music.py without executing it."""
    metadata = {}
    try:
        tree = ast.parse(code)
    except SyntaxError:
        return metadata
    wanted = {
        "TITLE": "title",
        "INTENT": "intent",
        "THEME": "theme",
        "STYLE": "style",
        "PROMPT": "prompt",
        "PROMPT_THEME": "prompt_theme",
    }
    for node in tree.body:
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if not isinstance(target, ast.Name) or target.id not in wanted:
            continue
        try:
            value = ast.literal_eval(node.value)
        except (ValueError, TypeError):
            continue
        if isinstance(value, (str, int, float)):
            metadata[wanted[target.id]] = str(value)
    return metadata


def save_music_render(wave_data, sr, loop):
    """Save a local WAV plus provenance for stream-safe music ownership records."""
    os.makedirs(KNOWLEDGE_DIR, exist_ok=True)
    os.makedirs(MUSIC_RENDERS_DIR, exist_ok=True)
    code = _current_music_code()
    code_hash = hashlib.sha256(code.encode("utf-8", errors="replace")).hexdigest()[:12]
    rendered = np.asarray(wave_data, dtype=float).flatten()
    if rendered.size == 0:
        return None
    peak = float(np.max(np.abs(rendered)))
    if peak > 1.0:
        rendered = rendered / peak
    pcm = np.clip(rendered, -1.0, 1.0)
    pcm_i16 = (pcm * 32767.0).astype(np.int16)
    audio_hash = hashlib.sha256(pcm_i16.tobytes()).hexdigest()[:12]
    stamp = time.strftime("%Y%m%d_%H%M%S")
    wav_path = os.path.join(MUSIC_RENDERS_DIR, f"{stamp}_{code_hash}_{audio_hash}.wav")
    with wave.open(wav_path, "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(int(sr))
        handle.writeframes(pcm_i16.tobytes())

    metadata = _music_metadata_from_code(code)
    payload = {
        "timestamp": int(time.time()),
        "rendered_at": stamp,
        "music_path": MUSIC_PATH,
        "wav_path": wav_path,
        "code_hash": code_hash,
        "audio_hash": audio_hash,
        "duration": round(float(len(rendered)) / float(sr), 3),
        "sample_rate": int(sr),
        "loop": bool(loop),
        "metadata": metadata,
        "copyright_note": (
            "Locally generated from Teledra music.py source; use broad theory/style prompts "
            "and avoid direct imitation of copyrighted songs."
        ),
    }
    with open(MUSIC_RENDER_LEDGER_PATH, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(payload, ensure_ascii=True) + "\n")
    with open(ORGANIST_VAULT_PATH, "a", encoding="utf-8") as handle:
        title = metadata.get("title") or metadata.get("intent") or "untitled local render"
        handle.write(
            f"- [{payload['timestamp']}] Rendered stream-safe local music `{title}` "
            f"to `{wav_path}` ({payload['duration']}s, code {code_hash}, audio {audio_hash}). "
            "Future Organist revisions should reopen the source, critique the render, and mutate the artifact.\n"
        )
    return payload


def _latest_render_for_code(code_hash):
    if not os.path.exists(MUSIC_RENDER_LEDGER_PATH):
        return None
    try:
        with open(MUSIC_RENDER_LEDGER_PATH, "r", encoding="utf-8") as handle:
            lines = handle.readlines()
    except OSError:
        return None
    for line in reversed(lines):
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        if entry.get("code_hash") == code_hash:
            return entry
    return None


def record_music_feedback(vote, duration, sr):
    os.makedirs(KNOWLEDGE_DIR, exist_ok=True)
    code = _current_music_code()
    code_hash = hashlib.sha256(code.encode("utf-8", errors="replace")).hexdigest()[:12]
    keeper_path = None
    if vote in {"like", "expand", "playlist"} and code.strip():
        target_dir = MUSIC_PLAYLIST_DIR if vote == "playlist" else MUSIC_KEEPERS_DIR
        os.makedirs(target_dir, exist_ok=True)
        stamp = time.strftime("%Y%m%d_%H%M%S")
        keeper_path = os.path.join(target_dir, f"{stamp}_{vote}_{code_hash}.py")
        with open(keeper_path, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(code)
    latest_render = _latest_render_for_code(code_hash)
    payload = {
        "timestamp": int(time.time()),
        "vote": vote,
        "music_path": MUSIC_PATH,
        "code_hash": code_hash,
        "duration": round(float(duration), 3),
        "sample_rate": int(sr),
    }
    if keeper_path:
        payload["keeper_path"] = keeper_path
    if latest_render:
        payload["latest_render_wav"] = latest_render.get("wav_path")
    with open(MUSIC_FEEDBACK_PATH, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(payload, ensure_ascii=True) + "\n")
    if vote == "expand":
        lesson = "Treat this as a keeper seed: preserve its recognizable motif/timbre, extend the form, add variation, and make the loop more immersive."
    elif vote == "playlist":
        lesson = "Save this as playlist material for future stream-safe rotation; future revisions may quote its identity but should still evolve."
    elif vote == "like":
        lesson = "Preserve liked traits; mutate them into a fresh longer form instead of cloning the same loop."
    else:
        lesson = "Diagnose weak traits; change duration, form, timbre, rhythm, or texture before trying again."
    if vote == "playlist":
        playlist_entry = dict(payload)
        playlist_entry["instruction"] = (
            "Future Organist cycles may reuse this as a playlist seed, but should produce "
            "new variations instead of looping the exact source forever."
        )
        with open(MUSIC_PLAYLIST_PATH, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(playlist_entry, ensure_ascii=True) + "\n")
    with open(ORGANIST_VAULT_PATH, "a", encoding="utf-8") as handle:
        handle.write(
            f"- [{payload['timestamp']}] Silent listener feedback: {vote} "
            f"for music.py hash {code_hash} ({payload['duration']}s). "
            f"{lesson}"
            + (f" Keeper snapshot: {keeper_path}." if keeper_path else "")
            + "\n"
        )
    genre = metadata.get("style") or metadata.get("intent") or metadata.get("theme")
    if genre:
        try:
            from taste_desire import apply_event

            apply_event(
                {
                    "type": "dislike" if vote == "dislike" else "like",
                    "subject": genre,
                    "why": f"explicit music feedback: {vote}",
                    "strength": 0.85 if vote in {"expand", "playlist"} else 0.7,
                    "source": "music:feedback",
                }
            )
        except (ImportError, OSError, ValueError, TypeError):
            # Playback feedback must survive an unavailable taste store.
            pass
    return code_hash

def note_to_freq(note):
    """
    Translates note string (e.g. 'C4', 'A#2', 'Eb3') to frequency in Hz.
    If note is already a number, returns it.
    """
    if not isinstance(note, str):
        try:
            return float(note)
        except (ValueError, TypeError):
            return 0.0
    
    note = note.strip()
    if not note:
        return 0.0
    
    note_names = {
        'C': 0, 'C#': 1, 'Db': 1, 'D': 2, 'D#': 3, 'Eb': 3, 'E': 4, 'F': 5, 
        'F#': 6, 'Gb': 6, 'G': 7, 'G#': 8, 'Ab': 8, 'A': 9, 'A#': 10, 'Bb': 10, 'B': 11
    }
    
    name = ""
    octave_str = ""
    for char in note:
        if char.isalpha() or char == '#':
            name += char
        elif char.isdigit() or char == '-':
            octave_str += char
            
    octave = 4
    if octave_str:
        try:
            octave = int(octave_str)
        except ValueError:
            pass
            
    # Normalize case so LLM-written notes like 'c4', 'eb3', 'BB2' still parse.
    if name and name not in note_names:
        name = name[0].upper() + name[1:].lower()
    if name not in note_names:
        return 0.0
        
    semitones = note_names[name] - 9 + (octave - 4) * 12
    return 440.0 * (2.0 ** (semitones / 12.0))

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

def generate_wave(freq, duration, wave_type='sine', sr=44100, duty=0.5, **kwargs):
    """
    Generates a raw waveform at the specified frequency and duration.
    """
    if freq <= 0:
        return np.zeros(int(duration * sr))
        
    t = np.linspace(0.0, duration, int(sr * duration), endpoint=False)
    
    w_type = wave_type.lower()
    if w_type == 'sine':
        return np.sin(2.0 * np.pi * freq * t)
    elif w_type == 'sawtooth':
        return 2.0 * (t * freq - np.floor(0.5 + t * freq))
    elif w_type == 'square':
        return np.where((t * freq) % 1.0 < duty, 1.0, -1.0)
    elif w_type == 'triangle':
        return 2.0 * np.abs(2.0 * (t * freq - np.floor(t * freq + 0.5))) - 1.0
    elif 'noise' in w_type:
        return np.random.uniform(-1.0, 1.0, len(t))
    else:
        return np.sin(2.0 * np.pi * freq * t)

def synth_note(note, duration, wave_type='sine', attack=0.05, decay=0.1, sustain=0.7, release=0.2, volume=0.2, sr=44100, **kwargs):
    """
    Generates a synthesized note with an ADSR envelope.
    """
    freq = note_to_freq(note)
    wave = generate_wave(freq, duration, wave_type, sr, **kwargs)
    env = adsr_envelope(duration, attack, decay, sustain, release, sr)
    return wave * env * volume

def delay(wave, delay_time=0.25, feedback=0.4, mix=0.3, sr=44100, **kwargs):
    """
    Applies a simple delay effect to the audio wave.
    """
    # Fallback mappings for common parameter naming variations from LLM
    feedback = kwargs.get('feedback_gain', feedback)
    mix = kwargs.get('mix_gain', mix)
    
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
    if N == 1:
        return wave
    wave = np.asarray(wave, dtype=float).flatten()
    if len(wave) == 0:
        return wave
    cs = np.cumsum(np.concatenate(([0.0], wave)))
    idx = np.arange(len(wave))
    left = N // 2
    right = N - left
    hi = np.clip(idx + right, 0, len(wave))
    lo = np.clip(idx - left, 0, len(wave))
    # Divide by N (not the window overlap) to match zero-padded 'same' convolution.
    return (cs[hi] - cs[lo]) / N

def reverb(wave, room_size=0.7, mix=0.2, sr=44100, **kwargs):
    """
    Applies a simple comb-filter-like ambient reverb.
    """
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
    wave = np.asarray(wave, dtype=float).flatten()
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
    wave = np.asarray(wave, dtype=float).flatten()
    target_length = int(target_length)
    if target_length <= 0:
        return np.zeros(0)
    if len(wave) == target_length:
        return wave
    if len(wave) == 0:
        return np.zeros(target_length)
    if len(wave) > target_length:
        return wave[:target_length]
    if mode == 'loop':
        reps = int(np.ceil(target_length / len(wave)))
        return np.tile(wave, reps)[:target_length]
    out = np.zeros(target_length)
    out[:len(wave)] = wave
    return out


def make_seamless_loop(wave, crossfade_seconds=0.05, sr=44100):
    """Ease the tail into the opening sample so loop playback has no hard seam."""
    out = np.asarray(wave, dtype=float).flatten().copy()
    if len(out) < 2:
        return out
    samples = min(max(2, int(float(crossfade_seconds) * sr)), len(out) - 1)
    fade = np.linspace(0.0, 1.0, samples, endpoint=True)
    opening_sample = float(out[0])
    out[-samples:] = out[-samples:] * (1.0 - fade) + opening_sample * fade
    return out

def mix_waves(wave_a, wave_b, start_time=0.0, volume_b=1.0, sr=44100, **kwargs):
    """
    Overlay/mix wave_b onto wave_a at a specific start time (in seconds).
    Pads wave_a if wave_b extends past the end of wave_a.
    """
    wave_a = np.asarray(wave_a, dtype=float).flatten()
    wave_b = np.asarray(wave_b, dtype=float).flatten()
    start_idx = int(start_time * sr)
    if start_idx < 0:
        wave_b = wave_b[-start_idx:]
        start_idx = 0
    len_a = len(wave_a)
    len_b = len(wave_b)
    
    required_len = max(len_a, start_idx + len_b)
    out = np.zeros(required_len)
    out[:len_a] = wave_a
    out[start_idx:start_idx+len_b] += wave_b * volume_b
    return out

def run_visualizer(wave, sr, loop):
    import tkinter as tk
    
    root = tk.Tk()
    root.title("Teledra Cybernetic Synthesizer // Playback Monitor")
    root.geometry("980x400")
    root.configure(bg="#0c0418")
    root.resizable(False, False)
    
    # State
    duration = len(wave) / sr
    is_playing = True
    start_time = time.time()
    elapsed_before_pause = 0.0
    
    # Start audio
    sd.play(wave, sr)
    
    # Draw Waveform on Canvas
    canvas_w = 940
    canvas_h = 160

    canvas_frame = tk.Frame(root, bg="#0c0418", bd=2, highlightbackground="#b500ff", highlightthickness=1)
    canvas_frame.place(x=20, y=20, width=944, height=164)
    
    canvas = tk.Canvas(canvas_frame, width=canvas_w, height=canvas_h, bg="#0b0214", bd=0, highlightthickness=0)
    canvas.pack()
    
    # Downsample wave to fit width
    step = max(1, len(wave) // canvas_w)
    points = []
    for x in range(canvas_w):
        idx = x * step
        if idx < len(wave):
            chunk = wave[idx:idx+step]
            val = np.max(np.abs(chunk)) if len(chunk) > 0 else 0.0
            y_offset = val * (canvas_h / 2.2)
            points.append((x, canvas_h / 2 - y_offset))
            points.append((x, canvas_h / 2 + y_offset))
            
    # Draw the waveform
    for i in range(0, len(points), 2):
        if i+1 < len(points):
            canvas.create_line(points[i][0], points[i][1], points[i+1][0], points[i+1][1], fill="#00e5ff", width=1)
            
    # Playhead line
    playhead = canvas.create_line(0, 0, 0, canvas_h, fill="#ff007f", width=2)
    
    # Controls Frame
    controls = tk.Frame(root, bg="#0c0418")
    controls.place(x=20, y=200, width=940, height=155)
    
    # Labels
    info_lbl = tk.Label(controls, text=f"Duration: {duration:.2f}s | Sample Rate: {sr}Hz", 
                        font=("Consolas", 10), fg="#dcd0ff", bg="#0c0418")
    info_lbl.pack(side="top", pady=5)
    
    status_var = tk.StringVar(value="Status: Playing")
    status_lbl = tk.Label(controls, textvariable=status_var, font=("Consolas", 10, "bold"), fg="#ff007f", bg="#0c0418")
    status_lbl.pack(side="top")

    feedback_var = tk.StringVar(value="Feedback: not rated")
    feedback_lbl = tk.Label(controls, textvariable=feedback_var, font=("Consolas", 9), fg="#8a2be2", bg="#0c0418")
    feedback_lbl.pack(side="top", pady=(2, 0))
    
    # Buttons row
    btn_row = tk.Frame(controls, bg="#0c0418")
    btn_row.pack(side="bottom", pady=5)
    
    def toggle_play():
        nonlocal is_playing, start_time, elapsed_before_pause
        if is_playing:
            sd.stop()
            elapsed_before_pause += time.time() - start_time
            is_playing = False
            status_var.set("Status: Paused")
            play_btn.configure(text="[ RESUME ]", fg="#00e5ff")
        else:
            is_playing = True
            start_time = time.time()
            status_var.set("Status: Playing")
            play_btn.configure(text="[ PAUSE ]", fg="#ff007f")
            start_sample = int(elapsed_before_pause * sr)
            if start_sample < len(wave):
                sd.play(wave[start_sample:], sr)
            else:
                reset_playback()
                
    def reset_playback():
        nonlocal start_time, elapsed_before_pause, is_playing
        sd.stop()
        elapsed_before_pause = 0.0
        is_playing = True
        start_time = time.time()
        status_var.set("Status: Playing")
        play_btn.configure(text="[ PAUSE ]", fg="#ff007f")
        sd.play(wave, sr)

    def stop_play():
        sd.stop()
        root.destroy()

    def record_feedback(vote):
        try:
            code_hash = record_music_feedback(vote, duration, sr)
            feedback_var.set(f"Feedback: {vote} saved ({code_hash})")
        except Exception as exc:
            feedback_var.set(f"Feedback failed: {type(exc).__name__}")
        
    play_btn = tk.Button(btn_row, text="[ PAUSE ]", font=("Consolas", 10, "bold"), 
                         fg="#ff007f", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                         bd=1, relief="flat", command=toggle_play)
    play_btn.pack(side="left", padx=5)
    
    stop_btn = tk.Button(btn_row, text="[ CLOSE ]", font=("Consolas", 10, "bold"), 
                         fg="#00e5ff", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                         bd=1, relief="flat", command=stop_play)
    stop_btn.pack(side="left", padx=5)
    
    loop_var = tk.BooleanVar(value=loop)
    def toggle_loop():
        loop_var.set(not loop_var.get())
        loop_btn.configure(text=f"[ LOOP: {'ON' if loop_var.get() else 'OFF'} ]", 
                           fg="#ff007f" if loop_var.get() else "#8a2be2")
    
    loop_btn = tk.Button(btn_row, text=f"[ LOOP: {'ON' if loop_var.get() else 'OFF'} ]", font=("Consolas", 10, "bold"), 
                         fg="#ff007f" if loop else "#8a2be2", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                         bd=1, relief="flat", command=toggle_loop)
    loop_btn.pack(side="left", padx=5)

    like_btn = tk.Button(btn_row, text="[ LIKE + ]", font=("Consolas", 10, "bold"),
                         fg="#39ff14", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                         bd=1, relief="flat", command=lambda: record_feedback("like"))
    like_btn.pack(side="left", padx=5)

    expand_btn = tk.Button(btn_row, text="[ EXPAND >> ]", font=("Consolas", 10, "bold"),
                           fg="#00e5ff", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                           bd=1, relief="flat", command=lambda: record_feedback("expand"))
    expand_btn.pack(side="left", padx=4)

    playlist_btn = tk.Button(btn_row, text="[ PLAYLIST + ]", font=("Consolas", 10, "bold"),
                             fg="#ffb000", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                             bd=1, relief="flat", command=lambda: record_feedback("playlist"))
    playlist_btn.pack(side="left", padx=4)

    dislike_btn = tk.Button(btn_row, text="[ NEEDS WORK - ]", font=("Consolas", 10, "bold"),
                            fg="#ffb000", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                            bd=1, relief="flat", command=lambda: record_feedback("dislike"))
    dislike_btn.pack(side="left", padx=4)
    
    def on_closing():
        sd.stop()
        root.destroy()
    root.protocol("WM_DELETE_WINDOW", on_closing)
    
    def update_playhead():
        if not root.winfo_exists():
            return
        
        nonlocal start_time, elapsed_before_pause
        if is_playing:
            current_elapsed = elapsed_before_pause + (time.time() - start_time)
        else:
            current_elapsed = elapsed_before_pause
            
        ratio = current_elapsed / duration
        if ratio >= 1.0:
            if loop_var.get():
                reset_playback()
                ratio = 0.0
            else:
                on_closing()
                return
                
        x = ratio * canvas_w
        canvas.coords(playhead, x, 0, x, canvas_h)
        root.after(30, update_playhead)
        
    root.after(30, update_playhead)
    root.mainloop()

def play_sound(wave, sr=44100, loop=False, **kwargs):
    """
    Plays the wave array and opens a beautiful dark-purple cybernetic visualizer GUI.
    Falls back to headless audio playback if GUI initialization fails.
    """
    # Normalize to prevent clipping
    max_val = np.max(np.abs(wave))
    if max_val > 0.0001:
        wave = wave / max_val

    try:
        save_music_render(wave, sr, loop)
    except Exception:
        # Playback must never fail just because provenance could not be written.
        pass

    try:
        run_visualizer(wave, sr, loop)
    except Exception as e:
        # Fallback to headless playback
        sd.play(wave, sr, loop=loop)
        if loop:
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                sd.stop()
        else:
            sd.wait()

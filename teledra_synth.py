import hashlib
import ast
import json
import os
import sys
import time
import wave

import numpy as np
import sounddevice as sd

def _diag(message):
    """Device/diagnostic banners go to stderr: stdout belongs to consumers
    (music_verify emits JSON on stdout and imports this module)."""
    print(message, file=sys.stderr)



def _apply_audio_device_override():
    """Let the operator pin Teledra's playback to a specific output device / host API.

    The default output here is often an HDMI/NVIDIA endpoint, which on Windows is
    frequently *single-client*: while Teledra loops a track it holds that endpoint,
    so VLC (especially on WASAPI-exclusive) can't open it and "won't launch
    properly." Routing Teledra to a different device -- or to a shared-mixing host
    API like DirectSound -- lets both play at once.

    Controlled by env vars (explicit) or auto-selection (sensible default):
      TELEDRA_AUDIO_DEVICE   = output device index, or a case-insensitive name substring,
                               or "default" / "system" / "windows" to follow the current
                               Windows default playback device exactly.
      TELEDRA_AUDIO_HOSTAPI  = host API name substring, e.g. "DirectSound" / "WASAPI" / "MME"
      TELEDRA_FOLLOW_WINDOWS_DEFAULT = 1   (alternative way to force following whatever
                                             is set as default in Windows Sound settings)

    When following Windows default, we do *not* auto-override to "better" devices.
    This lets you switch the default output device (or UNIFY bus) in Windows and have
    the synthesizer follow the same routing as other audio.

    Any failure here is swallowed -- audio must never break over a routing preference.
    """
    global _CHOSEN_OUTPUT_DEVICE
    want_dev = os.environ.get("TELEDRA_AUDIO_DEVICE", "").strip()
    want_api = os.environ.get("TELEDRA_AUDIO_HOSTAPI", "").strip().lower()
    follow_windows_default = (
        want_dev.lower() in ("", "default", "system", "windows", "os", "current") or
        os.environ.get("TELEDRA_FOLLOW_WINDOWS_DEFAULT", "").lower() in ("1", "true", "yes")
    )

    def _looks_virtual(name: str) -> bool:
        n = name.lower()
        if "unify" in n or "virtual" in n or "nvidia" in n or "samsung" in n or "hdmi" in n or "mapper - output" in n or "digital output" in n or "spdif" in n:
            return True
        # handle both ascii "rode" and real "røde"
        rode_like = ("rode" in n or "røde" in n or "roede" in n)
        if rode_like and not ("headphone" in n or "hodetelefoner" in n):
            return True
        return False

    def _is_good_hardware(name: str, channels: int) -> bool:
        n = name.lower()
        if channels < 2:
            return False
        good = ("speakers", "headphone", "headset", "realtek", "hd audio output")
        return any(g in n for g in good) and not _looks_virtual(name)

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
        if follow_windows_default:
            # Force using whatever is the current Windows default output right now.
            # This makes the synthesizer follow the same device/routing as normal audio
            # when the user switches the default playback device (including UNIFY buses).
            try:
                d = sd.default.device
                # sd.default.device may be _InputOutputPair (prints like list but isn't list/tuple)
                try:
                    cur_out = d[1]
                except (TypeError, IndexError):
                    cur_out = d if isinstance(d, int) else -1
                is_valid = isinstance(cur_out, int) and 0 <= cur_out < len(devices) and devices[cur_out].get('max_output_channels', 0) > 0
                if is_valid:
                    chosen = cur_out
                    _diag("[teledra_synth] forcing Windows default output device (following system routing)")
            except Exception as e:
                print('[DEBUG follow] exception:', e)
        elif want_dev:
            if want_dev.isdigit():
                idx = int(want_dev)
                if 0 <= idx < len(devices) and devices[idx]["max_output_channels"] > 0:
                    chosen = idx
            else:
                for i, d in enumerate(devices):
                    if (d["max_output_channels"] > 0
                            and want_dev.lower() in d["name"].lower()
                            and (api_idx is None or d["hostapi"] == api_idx)):
                        chosen = i
                        break

        # If user forced Realtek speakers but RØDE devices are present (common for interface users), override to a RØDE bus to avoid "muted" on wrong output
        # Skip when following Windows default (we want exactly what the user switched to).
        if not follow_windows_default and want_dev and "realtek" in want_dev.lower() and "speakers" in want_dev.lower():
            rode_indices = [i for i, d in enumerate(devices) if d.get("max_output_channels", 0) > 0 and (("rode" in d["name"].lower() or "røde" in d["name"].lower() or "unify" in d["name"].lower() or "hodetelefoner" in d["name"].lower()))]
            if rode_indices:
                # prefer hodetelefoner or system output
                for i in rode_indices:
                    nm = devices[i]["name"].lower()
                    if "hodetelefoner" in nm or "system output" in nm:
                        chosen = i
                        break
                if chosen is None:
                    chosen = rode_indices[0]
                _diag(f"[teledra_synth] Overriding forced Realtek speakers to RØDE device #{chosen} {devices[chosen]['name']} because RØDE interface is present (avoids muted audio on wrong output).")

        if chosen is None and api_idx is not None:
            default_out = hostapis[api_idx].get("default_output_device", -1)
            if default_out is not None and default_out >= 0:
                chosen = default_out

        # Auto-select a better audible device if no explicit want_dev and current default looks bad.
        # Skip entirely when we are explicitly following the Windows default.
        if chosen is None and not want_dev and not follow_windows_default:
            try:
                cur_out = sd.default.device[1] if isinstance(sd.default.device, (list, tuple)) else -1
                cur_name = devices[cur_out]["name"] if 0 <= cur_out < len(devices) else ""
                cur_ch = devices[cur_out]["max_output_channels"] if 0 <= cur_out < len(devices) else 0
                if _looks_virtual(cur_name) or cur_ch < 2:
                    candidates = []
                    for i, d in enumerate(devices):
                        ch = d.get("max_output_channels", 0)
                        if ch < 2:
                            continue
                        nm = d["name"]
                        if _looks_virtual(nm):
                            continue
                        score = 0
                        nml = nm.lower()
                        if "speakers" in nml:
                            score += 100
                        if "realtek" in nml:
                            score += 50
                        if "headphone" in nml or "headset" in nml:
                            score += 40
                        if "hodetelefoner" in nml:
                            score += 160  # very high priority for RØDE headphones (user's likely listening device)
                        if "unify" in nml and ("system" in nml or "music" in nml):
                            score += 140  # high for UNIFY monitoring buses, common audible path
                        if ("rode" in nml or "unify" in nml) and "mme" in ha:
                            score += 50  # prefer MME for RØDE/UNIFY, more reliable for the player
                        if ch >= 6:
                            score += 10
                        # Prefer WDM-KS / WASAPI for quality if tie
                        ha = hostapis[d["hostapi"]]["name"].lower() if d["hostapi"] < len(hostapis) else ""
                        if "wdm-ks" in ha or "wasapi" in ha:
                            score += 5
                        candidates.append((score, i, nm))
                    if candidates:
                        candidates.sort(reverse=True)
                        chosen = candidates[0][1]
                    if chosen is None:
                        # last resort: any non-virtual >=2ch
                        for i, d in enumerate(devices):
                            if d.get("max_output_channels", 0) >= 2 and not _looks_virtual(d["name"]):
                                chosen = i
                                break
            except Exception:
                pass

        if chosen is not None:
            sd.default.device = (sd.default.device[0] if isinstance(sd.default.device, (list, tuple)) else -1, chosen)
            d = sd.query_devices(chosen)
            ha_name = hostapis[d["hostapi"]]["name"] if d["hostapi"] < len(hostapis) else "?"
            _diag(f"[teledra_synth] audio output -> #{chosen} {d['name']} ({ha_name})")
            _CHOSEN_OUTPUT_DEVICE = chosen
            # Quick validation: try to open a tiny stream so we catch "invalid device" early
            def _validate_device(idx):
                try:
                    dummy = np.zeros((256, 2), dtype=np.float32)
                    sd.play(dummy, 44100, device=idx, blocking=False)
                    time.sleep(0.03)
                    sd.stop()
                    return True
                except Exception:
                    return False

            if not _validate_device(chosen):
                if follow_windows_default:
                    _diag(f"[teledra_synth] note: quick open test for Windows default #{chosen} failed, but following default as requested (no fallback).")
                    # do not change chosen
                else:
                    _diag(f"[teledra_synth] note: quick open test for #{chosen} failed (Invalid device). Searching for working alternative...")
                    # Fallback: look for other good analog-like devices that do validate. Prefer headphone-like if original choice was RØDE-related.
                    fallback = None
                    is_rode_choice = ("rode" in str(devices[chosen]["name"]).lower() or "røde" in str(devices[chosen]["name"]).lower() or "unify" in str(devices[chosen]["name"]).lower() or "hodetelefoner" in str(devices[chosen]["name"]).lower())
                    good_keywords = ("speakers", "headphone", "hodetelefoner", "monitor")
                    preferred = []
                    others = []
                    for i, d in enumerate(devices):
                        if d.get("max_output_channels", 0) < 2:
                            continue
                        nm = d["name"]
                        if _looks_virtual(nm):
                            continue
                        if any(kw in nm.lower() for kw in good_keywords):
                            if _validate_device(i):
                                if is_rode_choice and ("hode" in nm.lower() or "head" in nm.lower()):
                                    preferred.append(i)
                                else:
                                    others.append(i)
                    if preferred:
                        fallback = preferred[0]
                    elif others:
                        fallback = others[0]
                    if fallback is None:
                        # last resort any non-virtual high-channel
                        for i, d in enumerate(devices):
                            if d.get("max_output_channels", 0) >= 2 and not _looks_virtual(d["name"]):
                                if _validate_device(i):
                                    fallback = i
                                    break
                    if fallback is not None:
                        chosen = fallback
                        _CHOSEN_OUTPUT_DEVICE = chosen
                        d = devices[chosen]
                        ha_name = hostapis[d["hostapi"]]["name"] if d["hostapi"] < len(hostapis) else "?"
                        _diag(f"[teledra_synth] audio output -> #{chosen} {d['name']} ({ha_name})  [fallback after validation]")
                    else:
                        _diag("[teledra_synth] warning: could not find any validating output device.")
                        # Do not silently fall back to PC speakers (Realtek) if the user has RØDE devices - it will be the wrong physical output for them
                        if any(("rode" in devices[i]["name"].lower() or "røde" in devices[i]["name"].lower() or "unify" in devices[i]["name"].lower() or "hodetelefoner" in devices[i]["name"].lower()) for i in range(len(devices))):
                            _diag("[teledra_synth] info: RØDE/UNIFY devices present - not falling back to Realtek speakers (wrong output for most users with this interface).")
                            chosen = None  # force user to pick explicitly
        else:
            # Always surface the effective default so user can see why there might be no audio
            try:
                cur = sd.default.device[1] if isinstance(sd.default.device, (list, tuple)) else sd.default.device
                if isinstance(cur, int) and 0 <= cur < len(devices):
                    d = devices[cur]
                    ha_name = hostapis[d["hostapi"]]["name"] if d["hostapi"] < len(hostapis) else "?"
                    if _looks_virtual(d.get("name", "")):
                        _diag(f"[teledra_synth] audio output (default, possibly virtual/silent): #{cur} {d['name']} ({ha_name})")
            except Exception:
                pass
    except Exception as exc:
        _diag(f"[teledra_synth] audio device override ignored: {exc}")


# Module-level chosen device so we can force sd.play to the one we selected
# (setting sd.default.device alone is sometimes not enough for WDM-KS / certain devices)
_CHOSEN_OUTPUT_DEVICE = None

_apply_audio_device_override()

def _safe_play(audio, sr, **kwargs):
    """Wrapper that forces the device we chose in the override, if any."""
    if _CHOSEN_OUTPUT_DEVICE is not None:
        kwargs.setdefault("device", _CHOSEN_OUTPUT_DEVICE)
    return sd.play(audio, sr, **kwargs)

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
        "BPM": "bpm",
        "KEY": "key",
        "BARS": "bars",
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
    rendered = _audio_array(wave_data)
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
    channels = 1 if rendered.ndim == 1 else rendered.shape[1]
    with wave.open(wav_path, "wb") as handle:
        handle.setnchannels(channels)
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
        "channels": channels,
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
    metadata = _music_metadata_from_code(code)
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


def _audio_array(wave):
    """Return audio as frames or frames x channels without flattening stereo."""
    array = np.asarray(wave, dtype=float)
    if array.ndim == 0:
        return array.reshape(1)
    if array.ndim == 1:
        return array
    if array.ndim == 2:
        if array.shape[1] in (1, 2):
            return array
        if array.shape[0] in (1, 2):
            return array.T
    raise ValueError("audio must be mono or one/two-channel stereo")


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

    w_type = wave_type.lower().replace('-', '_').replace(' ', '_')
    if w_type == 'sine':
        return np.sin(2.0 * np.pi * freq * t)
    elif w_type == 'sawtooth':
        return 2.0 * (t * freq - np.floor(0.5 + t * freq))
    elif w_type == 'square':
        return np.where((t * freq) % 1.0 < duty, 1.0, -1.0)
    elif w_type == 'triangle':
        return 2.0 * np.abs(2.0 * (t * freq - np.floor(t * freq + 0.5))) - 1.0
    elif w_type in ('pink', 'pink_noise'):
        return pink_noise(
            duration,
            sr=sr,
            seed=kwargs.get('seed'),
            rng=kwargs.get('rng'),
        )
    elif 'noise' in w_type:
        rng = _noise_rng(seed=kwargs.get('seed'), rng=kwargs.get('rng'))
        return rng.uniform(-1.0, 1.0, len(t))
    else:
        return np.sin(2.0 * np.pi * freq * t)


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


PLAYBACK_PEAK_CEILING = 0.95


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

def run_visualizer(wave, sr, loop, geometry=None):
    import tkinter as tk

    root = tk.Tk()
    dev_name = ""
    try:
        if _CHOSEN_OUTPUT_DEVICE is not None:
            dd = sd.query_devices(_CHOSEN_OUTPUT_DEVICE)
            dev_name = dd["name"]
    except Exception:
        pass
    root.title(f"Teledra Cybernetic Synthesizer // {dev_name or 'Playback Monitor'}")
    if geometry:
        root.geometry(geometry)
    else:
        root.geometry("980x400")
    root.configure(bg="#0c0418")
    root.resizable(False, False)

    # State
    duration = len(wave) / sr
    is_playing = True
    start_time = time.time()
    elapsed_before_pause = 0.0

    # Note: audio start moved after GUI setup to allow status_var etc in error handling.

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
    dev_info = ""
    try:
        if _CHOSEN_OUTPUT_DEVICE is not None:
            dd = sd.query_devices(_CHOSEN_OUTPUT_DEVICE)
            dev_info = f" | Device: {dd['name']}"
    except Exception:
        pass
    info_lbl = tk.Label(controls, text=f"Duration: {duration:.2f}s | Sample Rate: {sr}Hz{dev_info}",
                        font=("Consolas", 10), fg="#dcd0ff", bg="#0c0418")
    info_lbl.pack(side="top", pady=5)

    if dev_name:
        dev_lbl = tk.Label(controls, text=f"OUTPUT: {dev_name}",
                           font=("Consolas", 11, "bold"), fg="#00ff9f", bg="#0c0418")
        dev_lbl.pack(side="top", pady=(0, 4))

    # Strong warning / guidance for RØDE UNIFY users (primary cause of "muted" Cybernetic Synthesizer)
    try:
        rode_present = any("rode" in d["name"].lower() or "unify" in d["name"].lower() or "hodetelefoner" in d["name"].lower() for d in sd.query_devices() if d.get("max_output_channels", 0) > 0)
        if rode_present:
            if "realtek" in dev_name.lower() or "speakers" in dev_name.lower():
                warn_lbl = tk.Label(controls, text="⚠️ WARNING: Using PC Realtek speakers. Your actual audio path is probably a RØDE UNIFY bus or Hodetelefoner headphones. Sound will be inaudible here. Run 'node strudel_app/app.mjs devices' and force the bus you heard the test tone on.",
                                    font=("Consolas", 9, "bold"), fg="#ffff00", bg="#3a0000", wraplength=920, justify="left")
                warn_lbl.pack(side="top", pady=5)
            else:
                # Always remind for virtual buses
                guide_lbl = tk.Label(controls, text="RØDE UNIFY: If no sound, open the RØDE UNIFY app now. Raise the fader for the bus shown above (OUTPUT line) and confirm it is routed to your headphones / monitors. Also raise python.exe in Windows Volume Mixer for that device.",
                                     font=("Consolas", 9), fg="#ffcc00", bg="#1a1200", wraplength=920, justify="left")
                guide_lbl.pack(side="top", pady=3)
    except Exception:
        pass

    status_var = tk.StringVar(value="Status: Playing (listen for 3 startup beeps now)")
    status_lbl = tk.Label(controls, textvariable=status_var, font=("Consolas", 10, "bold"), fg="#ff007f", bg="#0c0418")
    status_lbl.pack(side="top")

    # Start audio here, after GUI vars are defined, so error handling can use status_var
    # Play a LOUD multi-beep confirmation sequence first. This is the primary "is the device muted?" test.
    try:
        bsr = 44100
        def _beep_tone(freq, secs):
            bt = np.linspace(0, secs, int(bsr * secs))
            b = 0.82 * np.sin(2 * np.pi * freq * bt)
            return np.column_stack((b, b * 0.88))
        for f, secs in [(880, 0.28), (0, 0.12), (1200, 0.28), (0, 0.12), (1560, 0.32)]:
            if f > 0:
                _safe_play(_beep_tone(f, secs), bsr)
                time.sleep(secs + 0.08)
            else:
                time.sleep(secs)
        status_var.set("Status: Startup beeps played - if you heard them, device works. Music should follow.")
        _diag("[teledra_synth] startup BEEP sequence (3 tones) sent to chosen device")
    except Exception as e:
        status_var.set(f"Beep sequence error: {type(e).__name__}")
        _diag(f"[teledra_synth] startup beep sequence failed: {e}")

    try:
        _safe_play(wave, sr)
    except Exception as e:
        # Keep the GUI alive even if the main track fails to start; user can use BEEP TEST or VOL to diagnose
        status_var.set(f"Audio start failed: {type(e).__name__} - try BEEP TEST or different device")
        _diag(f"[teledra_synth] main track play failed on selected device: {e}")

    feedback_var = tk.StringVar(value="Feedback: not rated")
    feedback_lbl = tk.Label(controls, textvariable=feedback_var, font=("Consolas", 9), fg="#8a2be2", bg="#0c0418")
    feedback_lbl.pack(side="top", pady=(2, 0))

    # Buttons row
    btn_row = tk.Frame(controls, bg="#0c0418")
    btn_row.pack(side="bottom", pady=5)

    def get_scaled_wave():
        v = float(volume_var.get())
        return _protect_playback_peak(wave * v)

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
            scaled = get_scaled_wave()
            if start_sample < len(scaled):
                _safe_play(scaled[start_sample:], sr)
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
        _safe_play(get_scaled_wave(), sr)

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

    def play_test_beep():
        """Immediate audible test so user can verify the current device is not muted."""
        try:
            sd.stop()
            sr_local = 44100
            def _tb(freq, secs):
                tt = np.linspace(0, secs, int(sr_local * secs))
                b = 0.9 * np.sin(2 * np.pi * freq * tt)
                return np.column_stack((b, b * 0.85))
            # Triple confirm beep
            for ff, ss in [(950, 0.22), (0, 0.06), (1350, 0.22), (0, 0.06), (1650, 0.28)]:
                if ff > 0:
                    _safe_play(_tb(ff, ss), sr_local)
                    time.sleep(ss + 0.05)
                else:
                    time.sleep(ss)
            status_var.set("Status: BEEP TEST done - heard it? (if not: check UNIFY faders + Windows mixer for this device)")
        except Exception as e:
            status_var.set(f"Beep error: {type(e).__name__}")

    beep_btn = tk.Button(btn_row, text="[ BEEP TEST ]", font=("Consolas", 10, "bold"),
                         fg="#00ff9f", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                         bd=1, relief="flat", command=play_test_beep)
    beep_btn.pack(side="left", padx=5)

    def verify_notes_fire_gui():
        """Quick in-GUI verification that the synthesizer can produce distinct 'notes' (anti-stale check)."""
        try:
            sd.stop()
            sr_local = 44100
            def _ping(f, d=0.08):
                tt = np.linspace(0, d, int(sr_local * d))
                e = np.exp(-7 * np.linspace(0, 1, len(tt)))
                w = 0.88 * np.sin(2 * np.pi * f * tt) * e
                return np.column_stack((w, w * 0.82))
            status_var.set("VERIFY: playing distinct note pings (listen for each firing)...")
            base = 220
            for k in range(16):
                f = base + (k * 37) + (k % 3) * 11
                _safe_play(_ping(f), sr_local)
                time.sleep(0.105)
            status_var.set("VERIFY complete. Heard ~16 distinct pings? Notes can fire.")
        except Exception as e:
            status_var.set(f"Verify error: {type(e).__name__}")

    verify_btn = tk.Button(btn_row, text="[ VERIFY NOTES ]", font=("Consolas", 10, "bold"),
                           fg="#ffcc00", bg="#1b0a2a", activeforeground="#ffffff", activebackground="#5d3fd3",
                           bd=1, relief="flat", command=verify_notes_fire_gui)
    verify_btn.pack(side="left", padx=5)

    # === VOLUME CONTROL (to fix "muted" Cybernetic Synthesizer) ===
    vol_frame = tk.Frame(controls, bg="#0c0418")
    vol_frame.pack(side="bottom", pady=(2, 8))
    tk.Label(vol_frame, text="VOL", font=("Consolas", 9, "bold"), fg="#ff007f", bg="#0c0418").pack(side="left", padx=(0, 4))
    volume_var = tk.DoubleVar(value=1.0)
    def apply_volume(new_val=None):
        try:
            v = float(volume_var.get())
            sd.stop()
            scaled = _protect_playback_peak(wave * v)
            # restart from beginning with new volume
            nonlocal start_time, elapsed_before_pause, is_playing
            _safe_play(scaled, sr)
            elapsed_before_pause = 0.0
            start_time = time.time()
            is_playing = True
            status_var.set(f"Status: Playing (vol {v:.2f})")
        except Exception:
            pass
    vol_scale = tk.Scale(vol_frame, from_=0.0, to=2.5, resolution=0.05, orient=tk.HORIZONTAL,
                         variable=volume_var, command=lambda v: apply_volume(),
                         bg="#0c0418", fg="#dcd0ff", troughcolor="#4a2a6e", length=280, showvalue=1)
    vol_scale.pack(side="left")
    # initial play at default volume
    try:
        init_scaled = _protect_playback_peak(wave * volume_var.get())
        _safe_play(init_scaled, sr)
    except Exception:
        _safe_play(wave, sr)

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
    # Keep the composer's verified mix level and headroom. Only attenuate a mix
    # that exceeds the playback safety ceiling; never boost a quiet render.
    wave = _protect_playback_peak(wave)

    try:
        save_music_render(wave, sr, loop)
    except Exception:
        # Playback must never fail just because provenance could not be written.
        pass

    try:
        geo = os.environ.get("TELEDRA_WINDOW_GEOMETRY")
        run_visualizer(wave, sr, loop, geometry=geo)
    except Exception as e:
        # Fallback to headless playback (force selected device)
        _safe_play(wave, sr, loop=loop)
        if loop:
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                sd.stop()
        else:
            sd.wait()

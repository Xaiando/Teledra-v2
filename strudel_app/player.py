import json
import os
import sys
import time

import numpy as np

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)

from teledra_synth import (
    delay,
    fit_to_length,
    lowpass_filter,
    make_seamless_loop,
    mix_waves,
    play_sound,
    reverb,
    soft_limiter,
    stereo_pan,
    stereo_width,
    synth_note,
    _safe_play,
)


SR = 44100
CYCLE_SECONDS = 2.4


def drum_wave(name, duration):
    name = str(name).lower()
    duration = max(0.05, float(duration))
    if name in ("bd", "kick"):
        body = synth_note("C2", duration, wave_type="sine", attack=0.002, decay=0.08, sustain=0.0, release=0.18, volume=0.9)
        click = synth_note("C5", min(duration, 0.04), wave_type="noise", attack=0.001, decay=0.01, sustain=0.0, release=0.02, volume=0.12)
        return mix_waves(body, click, volume_b=1.0)
    if name in ("sd", "sn", "snare"):
        noise = synth_note("C4", duration, wave_type="white_noise", attack=0.002, decay=0.05, sustain=0.0, release=0.12, volume=0.45)
        tone = synth_note("D3", duration, wave_type="triangle", attack=0.001, decay=0.04, sustain=0.0, release=0.09, volume=0.18)
        return mix_waves(noise, tone, volume_b=1.0)
    if name in ("hh", "hat", "ch"):
        hat = synth_note("C6", min(duration, 0.11), wave_type="white_noise", attack=0.001, decay=0.02, sustain=0.0, release=0.05, volume=0.22)
        return lowpass_filter(hat, cutoff=8000.0)
    if name in ("cp", "clap"):
        clap = synth_note("C5", min(duration, 0.16), wave_type="white_noise", attack=0.001, decay=0.03, sustain=0.1, release=0.08, volume=0.28)
        return delay(clap, delay_time=0.018, feedback=0.45, mix=0.35)
    return synth_note("C4", min(duration, 0.18), wave_type="white_noise", attack=0.002, release=0.08, volume=0.12)


def event_wave(event):
    value = event.get("value", {})
    duration = max(0.05, (event["end"] - event["begin"]) * CYCLE_SECONDS)
    gain = float(value.get("gain", value.get("amp", 0.45)) or 0.45)
    attack = max(0.001, float(value.get("attack", 0.02) or 0.02))
    decay = max(0.001, float(value.get("decay", 0.08) or 0.08))
    sustain = float(np.clip(value.get("sustain", 0.65) or 0.65, 0.0, 1.0))
    release = max(0.01, float(value.get("release", 0.18) or 0.18))

    if "note" in value:
        note = value["note"]
        wave_type = str(value.get("s", "sine")).lower()
        if wave_type not in ("sine", "triangle", "sawtooth", "square"):
            wave_type = "sine"
        wave = synth_note(
            str(note),
            duration,
            wave_type=wave_type,
            attack=attack,
            decay=decay,
            sustain=sustain,
            release=release,
            volume=gain,
        )
    elif "s" in value:
        wave = drum_wave(value["s"], duration) * gain
    else:
        wave = np.zeros(int(duration * SR))

    cutoff = float(value.get("cutoff", value.get("lpf", 0.0)) or 0.0)
    if 20.0 < cutoff < SR / 2:
        wave = lowpass_filter(wave, cutoff=cutoff)
    delay_mix = float(np.clip(value.get("delay", 0.0) or 0.0, 0.0, 1.0))
    if delay_mix > 0.0:
        wave = delay(
            wave,
            delay_time=max(0.01, float(value.get("delaytime", 0.25) or 0.25)),
            feedback=float(np.clip(value.get("delayfeedback", 0.25) or 0.25, 0.0, 0.85)),
            mix=delay_mix,
        )
    room = float(np.clip(value.get("room", 0.0) or 0.0, 0.0, 1.0))
    if room > 0.0:
        wave = reverb(wave, room_size=room, mix=min(0.5, room * 0.55))
    return stereo_pan(wave, float(np.clip(value.get("pan", 0.0) or 0.0, -1.0, 1.0)))


def note_to_freq(note_name):
    """Rough note name to Hz for verification pings."""
    if not note_name:
        return 440.0
    n = str(note_name).lower().strip()
    base = {'c': 0, 'd': 2, 'e': 4, 'f': 5, 'g': 7, 'a': 9, 'b': 11}
    try:
        letter = n[0]
        octave = int(n[-1]) if n[-1].isdigit() else 4
        semis = base.get(letter, 0)
        if len(n) > 1 and n[1] in ('#', 's'):
            semis += 1
        elif len(n) > 1 and n[1] == 'b':
            semis -= 1
        midi = 12 * (octave + 1) + semis
        return 440.0 * (2 ** ((midi - 69) / 12.0))
    except Exception:
        return 440.0 + (hash(n) % 300)


def make_ping(freq, duration=0.10, vol=0.92):
    """Short distinct ping for verification that a 'note fired'."""
    t = np.linspace(0, duration, int(SR * duration), dtype=np.float32)
    env = np.exp(-6.5 * np.linspace(0, 1, len(t)))  # fast decay for clarity
    wave = vol * np.sin(2 * np.pi * freq * t) * env
    return np.column_stack((wave, wave * 0.82)).astype(np.float32)


def verify_notes_fire(events):
    """Verification layer requested: confirm that (almost) every scheduled note actually produces an audible event.

    Usage:
        $env:TELEDRA_VERIFY_NOTES=1
        node strudel_app/app.mjs play

    Plays a short, distinct, loud ping for each event in sequence (using pitch from the note when present).
    If you hear a ping roughly once per line printed, the notes are 'firing' in the synthesis code.
    If many are silent or weak, the generated Strudel pattern (gains, lpf, etc) may be stale/inaudible.
    """
    print("\n=== VERIFICATION LAYER: ALL NOTES FIRE ===")
    print(f"Total events: {len(events)}")
    if not events:
        print("No events to verify.")
        return

    print("Playing one distinct ping per event (spaced). Count the sounds.")
    print("Each printed line should correspond to one audible ping.")

    for idx, ev in enumerate(events):
        v = ev.get("value", {})
        note = v.get("note")
        smp = v.get("s", "")
        g = float(v.get("gain", v.get("amp", 0.5)) or 0.5)
        label = str(note or smp or "?")[:8]
        freq = note_to_freq(note) if note else (220 + (idx % 18) * 18 + (hash(smp) % 80))
        ping = make_ping(freq, duration=0.085, vol=min(0.97, 0.65 + g * 0.35))
        print(f"  {idx:3d} | {label:8s} | ~{freq:5.0f}Hz | gain={g:.2f}")
        try:
            _safe_play(ping, SR)
        except Exception:
            import sounddevice as sd
            sd.play(ping, SR)
        time.sleep(0.115)  # clear separation so each 'fires' is countable

    print("=== VERIFICATION COMPLETE ===")
    print("Heard a ping for nearly every event? Notes are firing.")
    print("Many missing/weak? Pattern generation likely produced low-energy or filtered-silent notes.")
    time.sleep(0.4)


def build_track(events):
    if not events:
        return np.zeros((int(CYCLE_SECONDS * SR), 2))

    end_cycle = max(float(event["end"]) for event in events)
    total_samples = int((end_cycle * CYCLE_SECONDS + 1.5) * SR)
    track = np.zeros((total_samples, 2))

    event_peaks = []
    for event in events:
        wave = event_wave(event)
        pk = float(np.max(np.abs(wave))) if len(wave) > 0 else 0.0
        event_peaks.append(pk)
        if pk < 0.015:
            print(f"[STALE?] very quiet event at t={event['begin']:.2f}: peak={pk:.5f} (may not be audible even with boost)")
        start_time = float(event["begin"]) * CYCLE_SECONDS
        track = mix_waves(track, wave, start_time=start_time, volume_b=1.0)

    if event_peaks:
        print(f"[VERIFY] event wave peaks: min={min(event_peaks):.4f} max={max(event_peaks):.4f} avg={sum(event_peaks)/len(event_peaks):.4f}")

    bed = synth_note("C2", CYCLE_SECONDS * max(1.0, end_cycle), wave_type="triangle", attack=1.2, decay=0.2, sustain=0.35, release=1.5, volume=0.035)
    bed = fit_to_length(lowpass_filter(bed, cutoff=550.0), len(track), mode="loop")
    track = mix_waves(track, stereo_pan(bed, 0.0), volume_b=0.38)
    track = delay(track, delay_time=0.31, feedback=0.22, mix=0.16)
    track = reverb(track, room_size=0.8, mix=0.24)
    track = stereo_width(track, 1.08)
    track = soft_limiter(track, drive=1.2, ceiling=0.92)
    return make_seamless_loop(track, crossfade_seconds=0.08, sr=SR)


def main():
    # Robust state_path: first non-option arg is the state file
    state_path = None
    for a in sys.argv[1:]:
        if not a.startswith("--"):
            state_path = a
            break
    if state_path is None:
        state_path = os.path.join(os.path.dirname(__file__), "state.json")
    with open(state_path, "r", encoding="utf-8") as f:
        state = json.load(f)
    if not state.get("ok"):
        raise RuntimeError(state.get("error") or "Cannot play invalid Strudel state.")

    events = state.get("events", [])
    verify = os.environ.get("TELEDRA_VERIFY_NOTES", "").lower() in ("1", "true", "yes", "on")

    # Parse optional geometry to anchor window (e.g. --x 1000 --y 500 or --geometry 980x400+1000+600)
    # to prevent music window overlapping Fractus. Can be set via env too from parent launch.
    import argparse
    p = argparse.ArgumentParser(add_help=False)
    p.add_argument("--x", type=int)
    p.add_argument("--y", type=int)
    p.add_argument("--geometry")
    extra_args, _ = p.parse_known_args(sys.argv[1:])
    if extra_args.geometry:
        os.environ["TELEDRA_WINDOW_GEOMETRY"] = extra_args.geometry
    elif extra_args.x is not None and extra_args.y is not None:
        os.environ["TELEDRA_WINDOW_GEOMETRY"] = f"980x400+{extra_args.x}+{extra_args.y}"

    if verify:
        verify_notes_fire(events)

    track = build_track(events)
    # Extra overall gain for strudel patterns (they tend to use conservative .gain() values)
    # Raised further because of RØDE virtual bus headroom / mixer issues
    track = np.clip(track * 2.0, -0.96, 0.96)

    if os.environ.get("TELEDRA_MONO", "").lower() in ("1", "true", "yes"):
        print("[AUDIO] TELEDRA_MONO=1 -> collapsing to mono for testing")
        mid = (track[:, 0] + track[:, 1]) * 0.5
        track = np.column_stack((mid, mid))
    print(f"STATUS:Local Strudel player rendering {len(events)} events")
    print("Starting audio + visualizer. Check the [teledra_synth] line above for the chosen output device.")
    print(">>> If the Cybernetic Synthesizer is MUTED: listen for the startup BEEP sequence (3 tones).")
    print(">>> Click the green [ BEEP TEST ] button in the GUI. If BEEP is silent too, the bus is not routed to your cans.")
    print(">>> To verify every note actually fires in the synthesis (stale pattern check):")
    print(">>>    $env:TELEDRA_VERIFY_NOTES=1 ; node strudel_app/app.mjs play")
    print(">>> To follow whatever is currently set as default in Windows (recommended for switching channels):")
    print(">>>    $env:TELEDRA_AUDIO_DEVICE=default ; node strudel_app/app.mjs play")
    print(">>>    or $env:TELEDRA_FOLLOW_WINDOWS_DEFAULT=1 ; node ...")
    print(">>> Then: 1) Open RØDE UNIFY app 2) Raise fader on the shown OUTPUT bus 3) Ensure it routes to your headphones.")
    print(">>> Or force a specific bus:  $env:TELEDRA_AUDIO_DEVICE='Music Output (RØDE UNIFY)'; $env:TELEDRA_AUDIO_HOSTAPI='MME'; node strudel_app/app.mjs play")
    play_sound(track, sr=SR, loop=True)


if __name__ == "__main__":
    main()

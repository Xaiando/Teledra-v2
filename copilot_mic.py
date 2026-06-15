"""Co-pilot mic listener: a long-running daemon that records short windows from
the default microphone, transcribes them with faster-whisper, and prints one
JSON line per detected utterance to stdout (mirrors restream_listener.py so the
Rust loop can read it as a stream of "you said ..." messages).

Usage:
    python copilot_mic.py [window_seconds]

stdout: {"text": "<utterance>"} per line.  stderr: STATUS lines.
"""

from __future__ import annotations

import json
import sys
import time

import numpy as np
import sounddevice as sd

SR = 16000
WINDOW = 5.0
RMS_GATE = 0.012  # skip near-silent windows so Whisper doesn't hallucinate text
MODEL_NAME = "base.en"


def log(msg: str) -> None:
    print(f"STATUS:{msg}", file=sys.stderr, flush=True)


def main() -> int:
    global WINDOW
    if len(sys.argv) > 1:
        try:
            WINDOW = max(2.0, float(sys.argv[1]))
        except ValueError:
            pass

    try:
        from faster_whisper import WhisperModel
    except Exception as exc:
        print(json.dumps({"error": f"faster-whisper not installed: {exc}"}), flush=True)
        return 1

    log(f"loading whisper {MODEL_NAME} (first run downloads the model)...")
    try:
        model = WhisperModel(MODEL_NAME, device="cpu", compute_type="int8")
    except Exception as exc:
        print(json.dumps({"error": f"whisper load failed: {exc}"}), flush=True)
        return 1
    log("mic listener ready")

    last_text = ""
    while True:
        try:
            audio = sd.rec(int(WINDOW * SR), samplerate=SR, channels=1, dtype="float32")
            sd.wait()
            audio = audio.flatten()
            if audio.size == 0:
                continue
            rms = float(np.sqrt(np.mean(audio ** 2)))
            if rms < RMS_GATE:
                continue
            segments, _info = model.transcribe(
                audio, language="en", vad_filter=True, beam_size=1
            )
            text = " ".join(seg.text.strip() for seg in segments).strip()
            if len(text) < 3:
                continue
            # Whisper on silence/noise often repeats canned phrases; drop dupes.
            if text.lower() == last_text.lower():
                continue
            last_text = text
            print(json.dumps({"text": text}, ensure_ascii=True), flush=True)
        except Exception as exc:
            log(f"mic loop error: {exc}")
            time.sleep(1.0)


if __name__ == "__main__":
    raise SystemExit(main())

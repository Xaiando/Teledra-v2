"""Capture one spoken phrase with an optional cross-platform or Windows backend."""

from __future__ import annotations

import json
import os
import subprocess
import sys

MAX_TEXT_CHARS = 16_000


def listen_with_faster_whisper() -> str:
    import numpy as np
    import sounddevice as sd
    from faster_whisper import WhisperModel

    sample_rate = 16_000
    seconds = float(os.environ.get("TELEDRA_MIC_WINDOW_SECS", "5"))
    seconds = min(20.0, max(2.0, seconds))
    audio = sd.rec(int(seconds * sample_rate), samplerate=sample_rate, channels=1, dtype="float32")
    sd.wait()
    audio = audio.reshape(-1)
    rms = float(np.sqrt(np.mean(audio ** 2))) if audio.size else 0.0
    if rms < 0.012:
        raise RuntimeError("microphone window was silent")
    model_name = os.environ.get("TELEDRA_WHISPER_MODEL", "base.en")
    model = WhisperModel(model_name, device="cpu", compute_type="int8")
    segments, _ = model.transcribe(audio, language="en", vad_filter=True, beam_size=1)
    return " ".join(segment.text.strip() for segment in segments).strip()


def listen_with_speech_recognition() -> str:
    import speech_recognition as sr

    recognizer = sr.Recognizer()
    with sr.Microphone() as source:
        audio = recognizer.listen(source, timeout=5, phrase_time_limit=20)
    return recognizer.recognize_google(audio)


def listen_with_windows_speech() -> str:
    if os.name != "nt":
        raise RuntimeError("the Windows speech fallback is only available on Windows")

    script = r"""
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
Add-Type -AssemblyName System.Speech
$installed = [System.Speech.Recognition.SpeechRecognitionEngine]::InstalledRecognizers()
if ($installed.Count -eq 0) { throw 'No Windows speech recognizer is installed.' }
$engine = [System.Speech.Recognition.SpeechRecognitionEngine]::new($installed[0])
try {
    $engine.LoadGrammar([System.Speech.Recognition.DictationGrammar]::new())
    $engine.SetInputToDefaultAudioDevice()
    $result = $engine.Recognize([TimeSpan]::FromSeconds(20))
    if ($null -eq $result -or [string]::IsNullOrWhiteSpace($result.Text)) {
        throw 'No speech was recognized before the timeout.'
    }
    [Console]::Write($result.Text)
} finally {
    $engine.Dispose()
}
"""
    completed = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
        check=False,
    )
    text = completed.stdout.strip()
    if completed.returncode != 0 or not text:
        detail = completed.stderr.strip() or "Windows speech recognition failed"
        raise RuntimeError(detail[-2_000:])
    return text


errors: list[str] = []
spoken_text = ""
backends = [listen_with_faster_whisper, listen_with_windows_speech]
if os.environ.get("TELEDRA_ALLOW_CLOUD_STT"):
    backends.append(listen_with_speech_recognition)
for backend in backends:
    try:
        spoken_text = backend().strip()[:MAX_TEXT_CHARS]
        if spoken_text:
            break
    except Exception as error:  # A failed optional backend should allow the fallback.
        errors.append(f"{backend.__name__}: {error}")

if spoken_text:
    response = {
        "ok": True,
        "capability": "microphone",
        "message": spoken_text,
        "data": {"text": spoken_text},
    }
else:
    response = {
        "ok": False,
        "capability": "microphone",
        "message": "; ".join(errors)[-4_000:] or "No microphone backend is available",
        "data": {},
    }

print(json.dumps(response, ensure_ascii=False))
sys.exit(0 if response["ok"] else 2)

"""The court's warm voice worker.

Loads the speech model once and then serves every utterance of the session
over stdin/stdout. Starting a fresh process per line is what made V1 stream
about twelve seconds of silence between sentences; the model load alone
measured 10.4 s and was paid again every time anyone spoke.

Protocol
--------
Requests arrive one per line on stdin::

    voice_slug<TAB>the text to speak

Audio is written to stdout as little-endian binary::

    i32  sample rate
    i32  sample count, then that many f32 samples   (repeated)
    i32  0                                          (end of utterance)

Everything human-readable goes to stderr, so the audio stream is never
polluted. ``READY`` on stderr means the model is resident and requests may
begin.

Two rules this file must keep
-----------------------------
1. **stdout carries audio and nothing else.** Libraries that print on import
   or during inference are redirected to stderr around every call.
2. **The reference encoding is cached per voice.** It is derived from a fixed
   recording and involves transcribing it; V1 redid that work for every single
   line spoken.
"""

from __future__ import annotations

import contextlib
import os
import struct
import sys

SAMPLE_RATE = 48_000
GAP_SECONDS = 0.16
READY = "READY"
MODEL = "YatharthS/LuxTTS"

# Where each voice's reference recording lives, relative to this file.
REFERENCES = {
    "queen": "../../assets/queen_ref_clean.wav",
    "organist": "../../assets/organist_ref_clean.wav",
    "alchemist": "../../assets/alchemist_ref_clean.wav",
    "archivist": "../../assets/archivist_ref_clean.wav",
    "orator": "../../assets/orator_ref_clean.wav",
    "scribe": "../../assets/scribe_ref_clean.wav",
    "artist": "../../assets/artist_ref_clean.wav",
    "diplomat": "../../assets/diplomat_ref_clean.wav",
    "treasurer": "../../assets/treasurer_ref_clean.wav",
}
FALLBACK_VOICE = "queen"


def note(message: str) -> None:
    """Says something to the operator without touching the audio stream."""
    print(message, file=sys.stderr, flush=True)


class AudioOut:
    """Writes the PCM framing described in this module's docstring."""

    def __init__(self, stream) -> None:
        self._stream = stream

    def begin(self) -> None:
        self._stream.write(struct.pack("<i", SAMPLE_RATE))

    def frame(self, samples) -> None:
        count = int(samples.size)
        if count <= 0:
            # Zero is the end marker, so an empty frame must never be sent.
            return
        self._stream.write(struct.pack("<i", count))
        self._stream.write(samples.astype("<f4", copy=False).tobytes(order="C"))

    def end(self) -> None:
        self._stream.write(struct.pack("<i", 0))
        self._stream.flush()


def reference_for(voice: str, base: str) -> str | None:
    """The reference recording for a voice, falling back to the Queen."""
    relative = REFERENCES.get(voice)
    if relative is None:
        note(f"WARNING: unknown voice '{voice}'; using {FALLBACK_VOICE}")
        relative = REFERENCES[FALLBACK_VOICE]

    path = os.path.normpath(os.path.join(base, relative))
    if os.path.exists(path):
        return path

    note(f"WARNING: no recording for '{voice}' at {path}; using {FALLBACK_VOICE}")
    fallback = os.path.normpath(os.path.join(base, REFERENCES[FALLBACK_VOICE]))
    if os.path.exists(fallback):
        return fallback

    note(f"ERROR: no reference recording at {fallback}")
    return None


def into_sentences(text: str, minimum_words: int = 5) -> list[str]:
    """Splits text for synthesis, keeping fragments joined.

    Very short pieces synthesised alone sound clipped and robotic, so a
    fragment is carried into its neighbour rather than spoken by itself.
    """
    import re

    pieces = [piece.strip() for piece in re.split(r"(?<=[.!?])\s+", text) if piece.strip()]
    if not pieces:
        return []

    grouped: list[str] = []
    pending = ""
    for piece in pieces:
        if not pending:
            pending = piece
        elif len(pending.split()) < minimum_words or len(pending) + len(piece) < 40:
            pending = f"{pending} {piece}"
        else:
            grouped.append(pending)
            pending = piece
    if pending:
        grouped.append(pending)
    return grouped


def find_luxtts() -> None:
    """Makes the speech library importable.

    LuxTTS is a third-party package with its own `pyproject.toml`, so the
    right answer is for it to be installed in the environment. V1 instead
    vendored a copy into the repository, which is why this needs a fallback at
    all. `TELEDRA_LUXTTS` may point at a source checkout; otherwise a sibling
    directory is tried, and an installed package needs neither.
    """
    try:
        import zipvoice  # noqa: F401

        return
    except ImportError:
        pass

    here = os.path.dirname(os.path.abspath(__file__))
    candidates = [
        os.environ.get("TELEDRA_LUXTTS", ""),
        os.path.normpath(os.path.join(here, "..", "..", "LuxTTS")),
        os.path.normpath(os.path.join(here, "..", "..", "..", "Teledra", "LuxTTS")),
    ]
    for candidate in candidates:
        if candidate and os.path.isdir(candidate):
            note(f"using the LuxTTS source at {candidate}")
            sys.path.insert(0, candidate)
            return
    note("WARNING: LuxTTS was not found; set TELEDRA_LUXTTS to its location")


class Voice:
    """The model, plus one cached reference encoding per voice."""

    def __init__(self, base: str) -> None:
        self._base = base
        self._prompts: dict[str, object] = {}
        # Importing torch prints to stdout; that would corrupt the audio.
        with contextlib.redirect_stdout(sys.stderr):
            find_luxtts()
            import torch
            from zipvoice.luxvoice import LuxTTS

            device = "cuda" if torch.cuda.is_available() else "cpu"
            note(f"loading the voice model on {device}")
            try:
                self._engine = LuxTTS(MODEL, device=device)
            except Exception as error:  # noqa: BLE001
                if device == "cpu":
                    raise
                # A busy or too-small GPU is a bad night, not a broken court.
                note(f"WARNING: {device} failed ({error}); falling back to CPU")
                self._engine = LuxTTS(MODEL, device="cpu", threads=2)

    def prompt_for(self, voice: str):
        """The encoded reference for a voice, computed at most once.

        Encoding transcribes the reference recording. That recording never
        changes, so doing it per utterance — as V1 did — is pure latency.
        """
        if voice in self._prompts:
            return self._prompts[voice]

        path = reference_for(voice, self._base)
        if path is None:
            return None
        note(f"encoding the reference for '{voice}' (once)")
        with contextlib.redirect_stdout(sys.stderr):
            prompt = self._engine.encode_prompt(path)
        self._prompts[voice] = prompt
        return prompt

    def warm_up(self, voice: str) -> None:
        """Pays the first-synthesis cost before anyone is listening.

        Encodes the reference and runs one short inference, discarding the
        audio. Measured, this moves roughly five seconds off the first line
        the court speaks.
        """
        import numpy as np

        prompt = self.prompt_for(voice)
        if prompt is None:
            return
        note("warming the voice")
        with contextlib.redirect_stdout(sys.stderr):
            wave = self._engine.generate_speech("Ready.   ", prompt)
        # Touch the result so nothing is optimised away, then drop it.
        _ = np.asarray(wave, dtype=np.float32).size

    def speak(self, voice: str, text: str, out: AudioOut) -> None:
        import numpy as np

        prompt = self.prompt_for(voice)
        out.begin()
        if prompt is None:
            # No recording: end cleanly so the court hears silence and reads
            # the reason on stderr, rather than waiting on a stalled pipe.
            out.end()
            return

        gap = np.zeros(int(SAMPLE_RATE * GAP_SECONDS), dtype=np.float32)
        sentences = into_sentences(text) or [text]
        for index, sentence in enumerate(sentences):
            spoken = sentence if sentence[-1:] in ".!?\"" else f"{sentence}."
            with contextlib.redirect_stdout(sys.stderr):
                wave = self._engine.generate_speech(f"{spoken}   ", prompt)
            samples = np.asarray(wave, dtype=np.float32).reshape(-1)
            out.frame(samples)
            if index + 1 < len(sentences):
                out.frame(gap)
        out.end()


def main() -> int:
    base = os.path.dirname(os.path.abspath(__file__))
    try:
        voice = Voice(base)
    except Exception as error:  # noqa: BLE001 - the court needs the reason
        note(f"ERROR: the voice model could not be loaded: {error}")
        return 1

    out = AudioOut(sys.stdout.buffer)

    # The first synthesis of a session costs several seconds more than the
    # rest — the reference is encoded and the inference path is compiled. Spend
    # that here, before announcing readiness, so the first line the audience
    # actually hears is as prompt as every line after it. READY then means
    # "ready to speak quickly", not merely "loaded".
    try:
        voice.warm_up(FALLBACK_VOICE)
    except Exception as error:  # noqa: BLE001
        note(f"WARNING: could not warm the voice: {error}")

    note(READY)

    for line in sys.stdin:
        line = line.rstrip("\r\n")
        if not line:
            continue
        if "\t" not in line:
            note("ERROR: a request must be 'voice<TAB>text'")
            continue

        voice_slug, text = line.split("\t", 1)
        voice_slug = voice_slug.strip().lower()
        text = text.strip()
        if not text:
            # Answer anyway: the court is waiting for exactly one utterance
            # per request and would otherwise block forever.
            out.begin()
            out.end()
            continue

        try:
            voice.speak(voice_slug, text, out)
        except Exception as error:  # noqa: BLE001
            # The reader is mid-utterance; it must be released or the whole
            # court stalls waiting for audio that will never arrive.
            note(f"ERROR: synthesis failed: {error}")
            out.end()

    return 0


if __name__ == "__main__":
    sys.exit(main())

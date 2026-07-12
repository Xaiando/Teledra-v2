import sys
import os
import contextlib
import struct
import wave
import numpy as np

# Add LuxTTS directory to path
sys.path.append(os.path.join(os.path.dirname(__file__), "LuxTTS"))

# Global cache for the LuxTTS instance
_lux_tts_instance = None
_torch_module = None
_lux_tts_class = None


# The Rust reader enforces the same limits. Keeping validation on both sides
# prevents a bad tensor shape, NaN, or runaway generation from becoming an
# unbounded pipe allocation before Rust has a chance to reject it.
MIN_SAMPLE_RATE = 8_000
MAX_SAMPLE_RATE = 192_000
MAX_FRAME_SECONDS = 90
MAX_TOTAL_SECONDS = 600
TRAILING_SILENCE_SECONDS = 0.35


class PCMProtocolError(RuntimeError):
    """Raised before malformed PCM can be written to the binary stdout pipe."""


class PCMStreamWriter:
    """Strict writer for Teledra's existing little-endian PCM pipe protocol.

    Wire format remains unchanged:
      int32 sample_rate
      repeated: int32 sample_count + sample_count float32 values
      int32 zero end marker
    """

    def __init__(
        self,
        stream,
        sample_rate,
        max_frame_samples=None,
        max_total_samples=None,
    ):
        sample_rate = int(sample_rate)
        if not MIN_SAMPLE_RATE <= sample_rate <= MAX_SAMPLE_RATE:
            raise PCMProtocolError(f"invalid sample rate {sample_rate}")
        self.stream = stream
        self.sample_rate = sample_rate
        self.max_frame_samples = int(
            max_frame_samples
            if max_frame_samples is not None
            else sample_rate * MAX_FRAME_SECONDS
        )
        self.max_total_samples = int(
            max_total_samples
            if max_total_samples is not None
            else sample_rate * MAX_TOTAL_SECONDS
        )
        if self.max_frame_samples <= 0 or self.max_total_samples <= 0:
            raise PCMProtocolError("PCM limits must be positive")
        self.total_samples = 0
        self.header_written = False
        self.finished = False

    def write_header(self):
        if self.header_written:
            raise PCMProtocolError("sample-rate header was written twice")
        if self.finished:
            raise PCMProtocolError("cannot write a header after the end marker")
        self.stream.write(struct.pack("<i", self.sample_rate))
        self.stream.flush()
        self.header_written = True

    def write_frame(self, samples):
        if not self.header_written:
            raise PCMProtocolError("cannot write PCM before the sample-rate header")
        if self.finished:
            raise PCMProtocolError("cannot write PCM after the end marker")

        frame = np.asarray(samples)
        if frame.ndim != 1:
            raise PCMProtocolError(f"PCM frame must be one-dimensional, got {frame.shape}")
        sample_count = int(frame.size)
        if sample_count <= 0:
            raise PCMProtocolError("zero samples are reserved for the end marker")
        if sample_count > self.max_frame_samples or sample_count > 2_147_483_647:
            raise PCMProtocolError(
                f"PCM frame has {sample_count} samples; limit is {self.max_frame_samples}"
            )
        new_total = self.total_samples + sample_count
        if new_total > self.max_total_samples:
            raise PCMProtocolError(
                f"PCM stream has {new_total} samples; limit is {self.max_total_samples}"
            )

        frame = np.ascontiguousarray(frame, dtype="<f4")
        if not np.isfinite(frame).all():
            bad_index = int(np.flatnonzero(~np.isfinite(frame))[0])
            raise PCMProtocolError(f"non-finite PCM sample at frame index {bad_index}")

        self.stream.write(struct.pack("<i", sample_count))
        self.stream.write(frame.tobytes(order="C"))
        self.total_samples = new_total

    def flush(self):
        self.stream.flush()

    def finish(self):
        if not self.header_written:
            raise PCMProtocolError("cannot finish before the sample-rate header")
        if self.finished:
            raise PCMProtocolError("PCM end marker was written twice")
        self.stream.write(struct.pack("<i", 0))
        self.stream.flush()
        self.finished = True


def _load_backend_modules():
    """Load heavyweight dependencies lazily so protocol tests stay lightweight."""
    global _torch_module, _lux_tts_class
    if _torch_module is None or _lux_tts_class is None:
        import torch

        # LuxTTS has a few plain print() calls. stdout is a binary protocol in
        # this process, so redirect every backend message to stderr.
        with contextlib.redirect_stdout(sys.stderr):
            from zipvoice.luxvoice import LuxTTS

        _torch_module = torch
        _lux_tts_class = LuxTTS
    return _torch_module, _lux_tts_class


REF_MAP = {
    "queen": "assets/queen_ref_clean.wav",
    "teledra": "assets/queen_ref_clean.wav",
    "energetic": "assets/queen_ref_clean.wav",
    "sarcastic": "assets/cenedra_ref_sarcastic.wav",
    "analytical": "assets/cenedra_ref_analytical.wav",
    "custom": "assets/cenedra_ref_custom.wav",
    "organist": "assets/organist_ref_clean.wav",
    "archivist": "assets/archivist_ref_clean.wav",
    "alchemist": "assets/alchemist_ref_clean.wav",
    "orator": "assets/orator_ref_clean.wav",
    "scribe": "assets/scribe_ref_clean.wav",
    "artist": "assets/artist_ref_clean.wav",
    "diplomat": "assets/diplomat_ref_clean.wav",
    "envoy": "assets/diplomat_ref_clean.wav",
    "treasurer": "assets/treasurer_ref_clean.wav",
    "wizard": "assets/wizard_ref_clean.wav",
    "default": "assets/queen_ref_clean.wav",
}

VOICE_PARAMS = {
    "default": {"rms": 0.0025, "speed": 1.02, "guidance_scale": 2.0, "t_shift": 0.7},
    "queen": {"rms": 0.0040, "speed": 1.16, "guidance_scale": 2.18, "t_shift": 0.80},
    "teledra": {"rms": 0.0040, "speed": 1.16, "guidance_scale": 2.18, "t_shift": 0.80},
    "energetic": {"rms": 0.0040, "speed": 1.16, "guidance_scale": 2.18, "t_shift": 0.80},
    "organist": {"rms": 0.0080, "speed": 1.06, "guidance_scale": 2.08, "t_shift": 0.73},
    "artist": {"rms": 0.0080, "speed": 1.06, "guidance_scale": 2.08, "t_shift": 0.73},
    "scribe": {"rms": 0.0065, "speed": 1.04, "guidance_scale": 2.06, "t_shift": 0.73},
    "archivist": {"rms": 0.0062, "speed": 1.04, "guidance_scale": 2.06, "t_shift": 0.73},
    "alchemist": {"rms": 0.0062, "speed": 1.05, "guidance_scale": 2.06, "t_shift": 0.73},
    "orator": {"rms": 0.0062, "speed": 1.07, "guidance_scale": 2.06, "t_shift": 0.73},
    "diplomat": {"rms": 0.0065, "speed": 1.06, "guidance_scale": 2.06, "t_shift": 0.73},
    "envoy": {"rms": 0.0065, "speed": 1.06, "guidance_scale": 2.06, "t_shift": 0.73},
    "treasurer": {"rms": 0.0065, "speed": 1.0, "guidance_scale": 2.06, "t_shift": 0.72},
    "wizard": {"rms": 0.0062, "speed": 0.98, "guidance_scale": 2.08, "t_shift": 0.72},
}


def split_for_tts(text, max_chars=220):
    """Split long generated sentences into clean phrase-sized TTS chunks."""
    import re

    raw_segments = [s.strip() for s in re.split(r'(?<!\w\.\w.)(?<![A-Z][a-z]\.)(?<=\.|\?|\!)\s+', text) if s.strip()]
    chunks = []

    for seg in raw_segments:
        if len(seg) <= max_chars:
            chunks.append(seg)
            continue

        parts = re.split(r'([,;:]\s+)', seg)
        current = ""
        for idx in range(0, len(parts), 2):
            phrase = parts[idx].strip()
            sep = parts[idx + 1].strip() if idx + 1 < len(parts) else ""
            if not phrase:
                continue
            piece = (phrase + sep).strip()
            candidate = (current + " " + piece).strip() if current else piece
            if len(candidate) <= max_chars:
                current = candidate
                continue
            if current:
                chunks.append(current)
            if len(piece) <= max_chars:
                current = piece
            else:
                words = piece.split()
                current = ""
                for word in words:
                    candidate = (current + " " + word).strip() if current else word
                    if len(candidate) <= max_chars:
                        current = candidate
                    else:
                        if current:
                            chunks.append(current)
                        current = word
        if current:
            chunks.append(current)

    return chunks

def get_lux_tts():
    global _lux_tts_instance
    if _lux_tts_instance is None:
        torch, lux_tts_class = _load_backend_modules()
        device = "cuda" if torch.cuda.is_available() else "cpu"
        try:
            print(f"INFO: Initializing LuxTTS on {device}...", file=sys.stderr)
            sys.stderr.flush()
            with contextlib.redirect_stdout(sys.stderr):
                if device == "cuda":
                    _lux_tts_instance = lux_tts_class('YatharthS/LuxTTS', device=device)
                else:
                    _lux_tts_instance = lux_tts_class(
                        'YatharthS/LuxTTS', device="cpu", threads=2
                    )
        except Exception as e:
            print(f"WARNING: Failed to initialize LuxTTS on {device}: {e}. Falling back to CPU.", file=sys.stderr)
            sys.stderr.flush()
            with contextlib.redirect_stdout(sys.stderr):
                _lux_tts_instance = lux_tts_class(
                    'YatharthS/LuxTTS', device="cpu", threads=2
                )
    return _lux_tts_instance


def prompt_duration_for_wav(path, target=8.0):
    """Use as much clean reference as possible without asking past the file end."""
    try:
        with wave.open(path, "rb") as wav:
            duration = wav.getnframes() / float(wav.getframerate())
    except Exception:
        return 5.0
    if duration <= 0:
        return 5.0
    return max(3.0, min(target, duration - 0.15))


def _resolve_ref_and_params(voice_name, base_dir, argv_tail):
    """Shared ref/params/override resolution for one-shot and resident paths."""
    ref_key = voice_name if voice_name in REF_MAP else "default"
    voice_params = VOICE_PARAMS.get(ref_key, VOICE_PARAMS["default"])
    if ref_key == "default" and voice_name not in ("default", "queen", "teledra", "energetic"):
        print(f"WARNING: Unknown voice '{voice_name}', falling back to Queen.", file=sys.stderr)
        sys.stderr.flush()
    ref_wav_path = os.path.join(base_dir, REF_MAP[ref_key])

    # A/B override support (env or 3rd positional in one-shot)
    override = os.environ.get("TELEDRA_VOICE_REF", "").strip()
    if argv_tail and argv_tail.strip():
        override = argv_tail.strip()
    if override:
        cand = override if os.path.isabs(override) else os.path.join(base_dir, override)
        if os.path.exists(cand):
            ref_wav_path = cand
            print(f"INFO: Using reference override {cand}", file=sys.stderr)
            sys.stderr.flush()
        else:
            print(f"WARNING: ref override {cand} not found; using {ref_wav_path}", file=sys.stderr)
            sys.stderr.flush()

    if not os.path.exists(ref_wav_path):
        print(f"WARNING: reference wav for '{voice_name}' missing at {ref_wav_path}; falling back to Queen.", file=sys.stderr)
        sys.stderr.flush()
        ref_wav_path = os.path.join(base_dir, "assets/queen_ref_clean.wav")
        if not os.path.exists(ref_wav_path):
            print(f"Error: reference wav not found at {ref_wav_path}", file=sys.stderr)
            return None, None, None
    return ref_wav_path, voice_params, ref_key


def _synthesize_to_protocol(text, voice_name, engine, ref_wav_path, voice_params):
    """Core synthesis used by both one-shot and resident modes. Writes full PCM protocol to stdout."""
    print("STATUS:Encoding reference voice...", file=sys.stderr)
    sys.stderr.flush()
    prompt_duration = prompt_duration_for_wav(ref_wav_path)
    print(f"INFO: Using {prompt_duration:.2f}s prompt from {os.path.basename(ref_wav_path)}", file=sys.stderr)
    sys.stderr.flush()

    with contextlib.redirect_stdout(sys.stderr):
        prompt_dict = engine.encode_prompt(
            ref_wav_path,
            duration=prompt_duration,
            rms=voice_params["rms"],
        )

    raw_segments = split_for_tts(text)

    # Group short segments together to prevent fragmented robotic cadence on short phrases
    sentences = []
    current = ""
    for seg in raw_segments:
        if not current:
            current = seg
        else:
            if len(current.split()) < 5 or len(seg.split()) < 3 or (len(current) + len(seg) < 40):
                current += " " + seg
            else:
                sentences.append(current)
                current = seg
    if current:
        sentences.append(current)

    if not sentences:
        sentences = [text]

    rate = 48000  # LuxTTS output rate is 48kHz
    silence_samples = int(rate * 0.16)
    silence = np.zeros(silence_samples, dtype=np.float32)

    protocol = PCMStreamWriter(sys.stdout.buffer, rate)
    protocol.write_header()

    num_sentences = len(sentences)
    for i, sentence in enumerate(sentences):
        if not sentence:
            continue
        print(f"PROGRESS:Synthesizing speech chunk {i+1} of {num_sentences}...", file=sys.stderr)
        sys.stderr.flush()

        padded_sentence = sentence.strip()
        if not padded_sentence.endswith(('.', '!', '?', '"')):
            padded_sentence += "."
        padded_sentence += "   "

        with contextlib.redirect_stdout(sys.stderr):
            wav_tensor = engine.generate_speech(
                padded_sentence,
                prompt_dict,
                num_steps=24,
                guidance_scale=voice_params["guidance_scale"],
                t_shift=voice_params["t_shift"],
                speed=voice_params["speed"]
            )
        segment = wav_tensor.detach().cpu().numpy().flatten().astype(np.float32)

        if i == len(sentences) - 1:
            trailing_silence = np.zeros(
                int(rate * TRAILING_SILENCE_SECONDS), dtype=np.float32
            )
            segment = np.concatenate([segment, trailing_silence])

        protocol.write_frame(segment)

        if i < len(sentences) - 1:
            protocol.write_frame(silence)

        protocol.flush()

    protocol.finish()


def synthesize_one_shot(text, voice_name, extra_arg=None):
    """One-shot entry for CLI / current callers. Returns False on fatal error."""
    base_dir = os.path.dirname(os.path.abspath(__file__))
    ref_wav_path, voice_params, _ = _resolve_ref_and_params(voice_name, base_dir, extra_arg)
    if ref_wav_path is None:
        return False

    try:
        print("STATUS:Loading voice models...", file=sys.stderr)
        sys.stderr.flush()
        engine = get_lux_tts()
        _synthesize_to_protocol(text, voice_name, engine, ref_wav_path, voice_params)
        return True
    except Exception as e:
        print(f"ERROR: Error generating voice via LuxTTS: {e}", file=sys.stderr)
        sys.stderr.flush()
        return False


def run_resident():
    """Resident / warm worker mode: model stays loaded; serves multiple requests over stdin/stdout.

    Request format (one line):
        voice_name<TAB>full text here
    Response: full PCM protocol stream (identical to one-shot) for the request,
              followed by "RESIDENT:RESPONSE_COMPLETE" on stderr.
    The process stays alive until stdin EOF.
    """
    engine = get_lux_tts()
    base_dir = os.path.dirname(os.path.abspath(__file__))
    print("RESIDENT:READY", file=sys.stderr)
    sys.stderr.flush()

    while True:
        line = sys.stdin.readline()
        if not line:
            # Parent closed the pipe
            break
        line = line.rstrip("\n\r")
        if not line:
            continue
        if "\t" not in line:
            print("ERROR: resident request must be 'voice_name<TAB>text'", file=sys.stderr)
            sys.stderr.flush()
            continue

        voice_name, text = line.split("\t", 1)
        voice_name = voice_name.strip().lower()
        text = text.strip()
        if not text:
            print("RESIDENT:RESPONSE_COMPLETE", file=sys.stderr)
            sys.stderr.flush()
            continue

        ref_wav_path, voice_params, _ = _resolve_ref_and_params(voice_name, base_dir, None)
        if ref_wav_path is None:
            print("RESIDENT:RESPONSE_COMPLETE", file=sys.stderr)
            sys.stderr.flush()
            continue

        try:
            _synthesize_to_protocol(text, voice_name, engine, ref_wav_path, voice_params)
            print("RESIDENT:RESPONSE_COMPLETE", file=sys.stderr)
            sys.stderr.flush()
        except Exception as e:
            print(f"ERROR: resident synthesis failed: {e}", file=sys.stderr)
            sys.stderr.flush()
            print("RESIDENT:RESPONSE_COMPLETE", file=sys.stderr)
            sys.stderr.flush()


def main():
    if "--resident" in sys.argv:
        run_resident()
        return

    if len(sys.argv) < 3:
        print("Usage: python generate_voice.py <text> <voice_name> [optional_ref_override]", file=sys.stderr)
        print("       python generate_voice.py --resident   # warm persistent worker", file=sys.stderr)
        sys.exit(1)

    text = sys.argv[1]
    voice_name = sys.argv[2].lower()
    extra = sys.argv[3] if len(sys.argv) >= 4 else None

    ok = synthesize_one_shot(text, voice_name, extra)
    if not ok:
        sys.exit(1)


if __name__ == "__main__":
    main()

import sys
import os
import wave
import numpy as np
import torch

# Add LuxTTS directory to path
sys.path.append(os.path.join(os.path.dirname(__file__), "LuxTTS"))
try:
    from zipvoice.luxvoice import LuxTTS
except ImportError as e:

    print(f"Error importing LuxTTS: {e}", file=sys.stderr)
    sys.exit(1)

# Global cache for the LuxTTS instance
_lux_tts_instance = None


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
        device = "cuda" if torch.cuda.is_available() else "cpu"
        try:
            print(f"INFO: Initializing LuxTTS on {device}...", file=sys.stderr)
            sys.stderr.flush()
            if device == "cuda":
                _lux_tts_instance = LuxTTS('YatharthS/LuxTTS', device=device)
            else:
                _lux_tts_instance = LuxTTS('YatharthS/LuxTTS', device="cpu", threads=2)
        except Exception as e:
            print(f"WARNING: Failed to initialize LuxTTS on {device}: {e}. Falling back to CPU.", file=sys.stderr)
            sys.stderr.flush()
            _lux_tts_instance = LuxTTS('YatharthS/LuxTTS', device="cpu", threads=2)
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


def main():
    if len(sys.argv) < 3:
        print("Usage: python generate_voice.py <text> <voice_name>", file=sys.stderr)
        sys.exit(1)
        
    text = sys.argv[1]
    voice_name = sys.argv[2].lower()
    
    # Resolve reference paths relative to this script so the spawning
    # process's working directory doesn't matter.
    base_dir = os.path.dirname(os.path.abspath(__file__))
    ref_key = voice_name if voice_name in REF_MAP else "default"
    voice_params = VOICE_PARAMS.get(ref_key, VOICE_PARAMS["default"])
    if ref_key == "default" and voice_name not in ("default", "queen", "teledra", "energetic"):
        print(f"WARNING: Unknown voice '{voice_name}', falling back to Queen.", file=sys.stderr)
        sys.stderr.flush()
    ref_wav_path = os.path.join(base_dir, REF_MAP[ref_key])

    # A/B override: TELEDRA_VOICE_REF env var or an optional 3rd CLI arg lets us
    # synthesize from an arbitrary reference (e.g. a cleaning candidate) without
    # touching REF_MAP. Default behaviour is unchanged when neither is set.
    override = os.environ.get("TELEDRA_VOICE_REF", "").strip()
    if len(sys.argv) >= 4 and sys.argv[3].strip():
        override = sys.argv[3].strip()
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
        # Fallback to the clean Queen reference if a specific voice does not exist.
        print(f"WARNING: reference wav for '{voice_name}' missing at {ref_wav_path}; falling back to Queen.", file=sys.stderr)
        sys.stderr.flush()
        ref_wav_path = os.path.join(base_dir, "assets/queen_ref_clean.wav")
        if not os.path.exists(ref_wav_path):
            print(f"Error: reference wav not found at {ref_wav_path}", file=sys.stderr)
            sys.exit(1)
        
    try:
        print("STATUS:Loading voice models...", file=sys.stderr)
        sys.stderr.flush()
        engine = get_lux_tts()
        
        print("STATUS:Encoding reference voice...", file=sys.stderr)
        sys.stderr.flush()
        # 1. Encode prompt from reference wav
        prompt_duration = prompt_duration_for_wav(ref_wav_path)
        print(f"INFO: Using {prompt_duration:.2f}s prompt from {os.path.basename(ref_wav_path)}", file=sys.stderr)
        sys.stderr.flush()
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
        silence_samples = int(rate * 0.16)  # Brief pause between generated speech segments
        silence = np.zeros(silence_samples, dtype=np.float32)
        
        # Write sample rate as 4-byte int first
        sys.stdout.buffer.write(np.array([rate], dtype=np.int32).tobytes())
        sys.stdout.buffer.flush()

        num_sentences = len(sentences)
        for i, sentence in enumerate(sentences):
            if not sentence:
                continue
            print(f"PROGRESS:Synthesizing speech chunk {i+1} of {num_sentences}...", file=sys.stderr)
            sys.stderr.flush()
            
            # Pad sentence to prevent LuxTTS from clipping the final syllable
            padded_sentence = sentence.strip()
            if not padded_sentence.endswith(('.', '!', '?', '"')):
                padded_sentence += "."
            padded_sentence += "   "

            wav_tensor = engine.generate_speech(
                padded_sentence, 
                prompt_dict, 
                num_steps=24, 
                guidance_scale=voice_params["guidance_scale"], 
                t_shift=voice_params["t_shift"], 
                speed=voice_params["speed"]
            )
            segment = wav_tensor.detach().cpu().numpy().flatten().astype(np.float32)
            
            # If it's the last sentence, pad it heavily with physical audio silence 
            # so the Rust `rodio` sink doesn't abruptly drop the OS audio buffer when it thinks it's done.
            if i == len(sentences) - 1:
                trailing_silence = np.zeros(int(rate * 1.20), dtype=np.float32)
                segment = np.concatenate([segment, trailing_silence])
            
            # Write segment sample count as 4-byte int, then segment audio data
            sys.stdout.buffer.write(np.array([len(segment)], dtype=np.int32).tobytes())
            sys.stdout.buffer.write(segment.tobytes())
            
            # If not the last sentence, append a brief silent pause segment
            if i < len(sentences) - 1:
                sys.stdout.buffer.write(np.array([len(silence)], dtype=np.int32).tobytes())
                sys.stdout.buffer.write(silence.tobytes())
                
            sys.stdout.buffer.flush()
            
        # Write 0 samples as an EOF marker
        sys.stdout.buffer.write(np.array([0], dtype=np.int32).tobytes())
        sys.stdout.buffer.flush()
    except Exception as e:
        print(f"Error generating voice via LuxTTS: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()

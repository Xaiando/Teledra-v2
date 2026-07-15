# Teledra Assets Manifest

The Teledra repository does not distribute multi-gigabyte models or audio assets.
To build and run Teledra natively, the following files and directories must exist relative to the project root:

## Python Environment
- `.venv/Scripts/python.exe`: Standard Python virtual environment.

## TTS & Audio Assets
- `voice_archive/`: A directory containing the reference voices for LuxTTS.
  - Required files include:
    - `queen_ref_clean.wav`
    - `artist_ref_clean.wav`
    - `organist_ref_clean.wav`
    - `alchemist_ref_clean.wav`
    - `archivist_ref_clean.wav`
    - `scribe_ref_clean.wav`
    - `diplomat_ref_clean.wav`
    - `orator_ref_clean.wav`
    - `treasurer_ref_clean.wav`
- `LuxTTS/`: Base folder for local LuxTTS models (or loaded via Hugging Face cache).

## Sidecars & Tooling
- `generate_voice.py`: PCM stream protocol generator script.
- `somatic_cortex_stream.py`: Screen vision and microphone integration sidecar.

If any of these assets or dependencies are missing, `main.rs` will halt execution explicitly upon startup.

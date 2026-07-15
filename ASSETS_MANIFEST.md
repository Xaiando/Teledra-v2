# Teledra Assets Manifest

The Teledra repository does not distribute multi-gigabyte models or audio assets.
To build and run Teledra natively, the following files and directories must exist relative to the project root:

## Python Environment
- `.venv/Scripts/python.exe`: Standard Python virtual environment.

## Configuration
- `config.qwen.json`: Local Ollama Qwen model configuration.

## TTS & Audio Assets
- `assets/`: A directory containing the reference voices for LuxTTS.
  - Required files include:
    - `assets/queen_ref_clean.wav`
    - `assets/cenedra_ref_sarcastic.wav`
    - `assets/cenedra_ref_analytical.wav`
    - `assets/cenedra_ref_custom.wav`
    - `assets/organist_ref_clean.wav`
    - `assets/archivist_ref_clean.wav`
    - `assets/alchemist_ref_clean.wav`
    - `assets/orator_ref_clean.wav`
    - `assets/scribe_ref_clean.wav`
    - `assets/artist_ref_clean.wav`
    - `assets/diplomat_ref_clean.wav`
    - `assets/treasurer_ref_clean.wav`
    - `assets/wizard_ref_clean.wav`
- `LuxTTS/`: Base folder for local LuxTTS models (or loaded via Hugging Face cache).

## Sidecars & Tooling
- `generate_voice.py`: PCM stream protocol generator script.
- `somatic_cortex_stream.py`: Screen vision and microphone integration sidecar.

If any of these assets or dependencies are missing, `main.rs` will fail explicitly upon startup if run in strict mode.

# Teledra

Welcome to Teledra, a generative sovereign court orchestration platform.

## Configuration

Teledra requires a JSON configuration file for the internal LLM bridge (Brain). You can specify the config file path by setting the `$env:TELEDRA_CONFIG` environment variable. If not set, Teledra defaults to `config.json`.

```powershell
$env:TELEDRA_CONFIG = 'config.qwen.json'
```

If the specified configuration points to a local or non-authenticated model (e.g. Ollama `http://localhost:11434`), an empty `api_key` is permitted.

## Launch Modes

Teledra supports two launch modes:

1. **Minimal Mode** (`--minimal`): This is the default. Missing sidecars (such as Somatic/Vision/Voice) are gracefully disabled, and Teledra runs with limited capabilities.
2. **Strict Mode** (`--strict`): Enforces that all assets, dependencies, and sidecars are fully available before launching. If anything is missing, the application will exit immediately.

You can verify the environment and active capabilities before launching the TUI by running:

```powershell
cargo run --release -- --check-environment
```

To launch the court:

```powershell
cargo run --release -- --minimal
# or
cargo run --release -- --strict
```

## Prerequisites

To successfully start Teledra, your machine **must** have:

1. **Rust Toolchain**: version 1.95.0. 
   *(This is defined automatically via `rust-toolchain.toml`).*

2. **Python Environment**: 
   Teledra requires a local virtual environment located at `.venv` inside the project root. Install dependencies using the grouped requirements files depending on your needs (e.g. `requirements-core.txt`, `requirements-all.txt`).
   
   ```powershell
   python -m venv .venv
   .\.venv\Scripts\Activate.ps1
   pip install -r requirements-all.txt
   ```

3. **External Assets**: 
   Various generative models and `.wav` templates are required for voice. See `ASSETS_MANIFEST.md` for exact file paths. Note that in minimal mode, missing assets will merely disable the voice capability.

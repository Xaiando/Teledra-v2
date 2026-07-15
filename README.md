# Teledra

Welcome to Teledra, a generative sovereign court orchestration platform.

## Minimal Launch Path

The only supported method for compiling and launching Teledra natively is via `cargo`:

```powershell
cargo run --release
```

## Prerequisites

To successfully start Teledra, your machine **must** have:

1. **Rust Toolchain**: version 1.95.0. 
   *(This is defined automatically via `rust-toolchain.toml`, so standard `cargo` installations will use the right version).*

2. **Python Environment**: 
   Teledra requires a local virtual environment located at `.venv` inside the project root, with all dependencies installed.
   
   ```powershell
   python -m venv .venv
   .\.venv\Scripts\Activate.ps1
   pip install -r requirements.txt
   ```

3. **External Assets**: 
   Various generative models and `.wav` templates are required. See `ASSETS_MANIFEST.md` for exact file paths and instructions on where they must reside.
   If these files are missing, the court will **panic immediately** rather than running silently broken.

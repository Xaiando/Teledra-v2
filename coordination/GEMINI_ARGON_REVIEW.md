# Gemini Argon inspection handoff — 14 October 2026

## Task for the reviewer

Inspect this repository as a working local application, with particular attention
to correctness, process ownership, bounded model execution, creative artifact
validation, and claims that are stronger than the available evidence. Produce
prioritized findings with file/line references, concrete reproduction steps,
and proposed repairs. Distinguish confirmed bugs from hypotheses. Do not treat
historical acceptance documents or passing unit tests as proof of current live
audio quality, musical quality, continuous conversation, or safe shutdown.

This handoff supplies source for inspection; it does not connect Gemini to the
machine or send it credentials. Start with this guide, then inspect source.

## Which version this is

This is the modular Rust workspace in `D:\Teledra V2`, not the older monolithic
application in `D:\Teledra`. The GitHub URL is
<https://github.com/Xaiando/Teledra-v2/tree/main>.

Before this synchronization, GitHub main was
`2303ff3d12be1c5dd96298efd56e9dad8c726d7e` and had an unrelated history and
monolithic layout. Its exact tip remains available at
`codex/pre-local-v2-sync-20261014`. An explicit merge using the local tree
preserves both histories. Remote-only files and its Windows CI workflow were
not transplanted into the modular rewrite. No force push is required.

The source ZIP and text cargo are exported from committed Git blobs, not from
an uncontrolled directory copy. Their filenames and manifest identify the
exact published commit. The manifest inventories every file with SHA-256.

## Reading map

| Area | Start here | Questions to inspect |
| --- | --- | --- |
| Workspace and launch | `Cargo.toml`, `rust-toolchain.toml`, `Setup-Python.ps1`, `Teledra.bat` | Clean checkout dependencies, launch/build behavior, Windows assumptions |
| Court scheduling | `Teledra/src/main.rs`, `app.rs`, `podcast.rs`, `session.rs` | Interruptions, locked topics, stale replies, fairness, repeated prompts |
| Model boundary | `Teledra/Brain/src/lib.rs`, `prompt_fit.rs`, `Court/src/lib.rs`, `actions.rs` | Response bounds, timeouts, parsing, corrections, schema enforcement |
| Runtime actions | `Teledra/src/runtime/mod.rs`, `actions.rs`, `commands.rs`, `artifacts.rs` | Model requests versus operator authority, cancellation, file writes |
| Python music | `Music/PYTHON_AUTHORING.md`, `python_music_worker.py`, `python_music_editor.py`, `Teledra/src/python_*.rs`, `runtime/pymusic.rs` | Generated-code execution, validation, note fidelity, repair bounds |
| Native music | `Music/src/lib.rs`, `composer/`, `theory.rs`, `arrange.rs`, `synth/`, `effects.rs` | Score semantics, audible notes, renderer verification, headroom |
| Evolution and learning | `runtime/music_edits.rs`, `music_learning.rs`, `music_study.rs`, `music_library.rs`, `art_learning.rs` | Genuine changes, listener evidence, transactional persistence |
| Fractus | `Fractus/src/lib.rs`, `kinetic.rs`, `orbits.rs`, `richness.rs`, `studio.py`, `runtime/fractus.rs` | Geometry changes, gallery retention, timing, validation versus quality |
| Speech and lifecycle | `TTS/src/runtime.rs`, `worker.rs`, `voice.rs`, `TTS/worker/voice_worker.py`, `Teledra/src/lifetime.rs`, `Sidecars/scripts/studio_lifetime.py` | Framing, stale playback, interruption, worker death, child cleanup |
| External inputs | `Research/src/lib.rs`, `Sidecars/scripts/youtube.py`, `runtime/chat_listener.rs`, `audience.rs`, `sidecars.rs` | Untrusted content, provenance, credential handling, reconnect behavior |
| Missions and tools | `Swarm`, `Frankenswarm`, `Kraken`, `Tower`, `Workshop` | Durable state, evidence-bound completion, paths, sandbox boundaries |
| Separate experiment | `FlyComposer/README.md`, `ATTRIBUTION.md`, `flycomposer/`, `tests/` | Engineered versus biological claims, rendering, training provenance |

Read `README.md` and the subsystem READMEs for operator features. Consult
`coordination/HANDOVER.md`, `ASTRA_OVERHAUL.md`, `OPERATIONAL_ACCEPTANCE.md`,
`SPEECH_CONTINUITY_20260917.md`, `YOUTUBE_PODCAST_20260918.md`, and the music and
Fractus September notes for historical decisions. `WIZARD_KIT_20260923.md`
describes work in the older project's cloud resident, whose implementation is
not included here; do not attribute that implementation to this workspace.

## Reproduce locally

Windows requires Rust 1.95.0 and Visual Studio C++ Build Tools. Model inference
uses local Ollama. The default model is `qwen2.5:7b`, but
`Teledra/Brain/src/lib.rs::model_name()` honors `TELEDRA_MODEL` once per process.
The packaging machine sets it to `qwen2.5-coder:14b`; the root README's statement
that the model is fixed is therefore stale. This is an inspection target.

```powershell
cargo test --workspace --locked --no-fail-fast
cargo build --release --workspace --locked
powershell -NoProfile -ExecutionPolicy Bypass -File .\Setup-Python.ps1
.\Teledra.bat --check-environment
```

Tests with hard-coded model expectations need the default environment. To
reproduce that baseline, clear `TELEDRA_MODEL` only in the test shell. The
application supports an override; a non-default environment should be audited
separately rather than silently treated as a default-model run. Some tests are
explicitly ignored because they need Ollama, audio devices, or a live desktop.
Provisioning downloads dependencies/models; it is not required for reading.

## Snapshot boundaries

Included: the Rust workspace and lockfile, Python source/tests, documentation,
reference voice assets, selected FlyComposer brain/circuit/foundation data and
cached provenance pages. Secrets and local runtime directories are excluded.

Excluded and retained locally: `target/`, virtual environments, `.teledra/`,
AppData court records, local credentials, FlyComposer library/reports/raw data,
`Music/training/`, `FlyComposer/training/`, downloaded foundation source music,
`evolution-proof/`, and the prebuilt `FlyComposer/bin/PyMusicSynth.exe`.
Links into those excluded run/report folders may therefore be unavailable.
The ZIP is an inspection snapshot, not a complete archive of listening or
training evidence. Trained NPZ files are included for inspection.

FlyComposer's packaged native note helper is a build output and is omitted.
For local use, build `cargo build --release -p teledra --locked` at the workspace
root, then copy `target/release/teledra.exe` to
`FlyComposer/bin/PyMusicSynth.exe` (create the directory first). The application
supports `--music-note-server`. A rebuilt helper will not match the historical
binary fingerprint in `FlyComposer/ATTRIBUTION.md`; validate it before making
claims about reproducing old recordings.

## Verification during this handoff

Fresh verification results are recorded in `ARGON_VALIDATION.md`. Prior
acceptance reports are dated historical evidence. The handoff does not assert
that hardware-dependent or live model acceptance was repeated.

## Generate the review package

After committing the tree, run `python coordination/build_argon_cargo.py`.
It creates an ignored `handoff-output/` folder containing the source ZIP,
one UTF-8 text cargo with source-file boundaries, and a JSON file inventory
and artifact checksums. Supply the ZIP to a file-aware reviewer, or the text
cargo to a reviewer accepting a large text attachment. Give this instruction:

> Read coordination/GEMINI_ARGON_REVIEW.md first. Audit the current committed
> modular V2 implementation. Prioritize actionable correctness and lifecycle
> findings, cite files and lines, and separate measured facts from assumptions.
> Use ARGON_VALIDATION.md as the current check record; do not infer live quality
> from historical reports or mocked tests.

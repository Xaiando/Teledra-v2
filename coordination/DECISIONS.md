# Teledra V2 decisions

## 2026-07-27 — Codex

- The original tree is reference-only; migration is selective.
- Qwen `qwen2.5:7b` is a compile-time invariant, not a configurable choice.
- Ollama's native `/api/chat` contract is the sole model transport.
- Court-member histories are isolated so role details can evolve independently.
- V2 begins as a native terminal application. No local browser UI is introduced.
- Generated output and runtime state live outside maintained source.

## 2026-07-27 — Codex, TUI acceptance correction

- A line-oriented CLI is not an acceptable application shell for the V2 court.
- The application is a native Rust TUI with rendering, state, terminal
  lifecycle, and asynchronous coordination kept in separate modules.
- A Qwen reply is stored in the chamber that originated it even if the operator
  changes the visible court member while inference is running.
- Only one court inference runs at a time until subsystem-level scheduling is
  designed explicitly.

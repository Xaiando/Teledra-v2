# Teledra V2 decisions

## 2026-07-27 — Codex

- The original tree is reference-only; migration is selective.
- Qwen `qwen2.5:7b` is a compile-time invariant, not a configurable choice.
- Ollama's native `/api/chat` contract is the sole model transport.
- Court-member histories are isolated so role details can evolve independently.
- V2 begins as a native terminal application. No local browser UI is introduced.
- Generated output and runtime state live outside maintained source.


# Teledra V2 migration map

## Rules

1. `D:\Teledra` remains untouched and is used only as behavioral reference.
2. V2 supports exactly `qwen2.5:7b`. There is no provider abstraction, remote
   API key, alternate chat model, or separate code model.
3. State, generated artifacts, caches, and experiment output are data, not
   source code. They are not migrated into the repository.
4. A feature moves only with a boundary, tests, and a documented owner.
5. The native terminal remains usable after every migration slice.

## Ownership map

| V2 folder | Owns | V1 reference areas | Status |
| --- | --- | --- | --- |
| `Teledra/Brain` | Qwen request/response boundary | `src/brain.rs` | Working core |
| `Agents` | Court identities and prompts | role prompts in `src/brain.rs` | Working core |
| `Court` | Sessions, turns, court policy | court routing in `src/main.rs` | Working core |
| `Teledra` | Native application shell and composition root | TUI/runtime in `src/main.rs` | Working core |
| `TTS` | Speech queue, voices, playback | `src/voice.rs`, `generate_voice.py` | Not migrated |
| `Music` | Scores, rendering, feedback workflow | `court_synth/`, music sections of `src/main.rs` | Not migrated |
| `Fractus` | Visual scene DSL and renderer boundary | `Fractus/` | Not migrated |
| `Swarm` | General bounded task execution | `swarm/` | Not migrated |
| `Frankenswarm` | Experimental swarm composition | `Frankenswarm/` | Not migrated |
| `Kraken` | Code/game verification harness | `kraken/` | Not migrated |
| `Tower` | Isolated residents and remote reports | `tower/`, `cloud_residents/` | Not migrated |

## Planned slices

1. Qwen-only conversation core and role-local court sessions.
2. Durable court transcript and explicit event contracts.
3. TTS as an optional consumer of completed court utterances.
4. Music as an independent score/render service with immutable feedback events.
5. Fractus visuals behind the same event boundary.
6. Swarm, Kraken, Frankenswarm, and Tower only after their contracts can be
   described without importing the old monolith.

## Explicitly rejected legacy patterns

- A monolithic application file that owns UI, prompts, persistence, tools,
  music, and process supervision.
- Model selection in configuration.
- Runtime-generated source files mixed with maintained source.
- Hard-coded absolute paths.
- Implicit cross-subsystem file writes.
- Copying caches, run output, databases, binaries, or abandoned experiments.


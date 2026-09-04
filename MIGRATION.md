# Teledra V2 migration map

## Rules

1. `D:\Teledra` remains untouched and is used only as behavioral reference.
2. V2 supports exactly local Ollama `qwen2.5:7b`.
3. Runtime state and generated artifacts live outside the source tree.
4. Every capability has one small owner, bounded contracts, and tests.
5. Slow, networked, or device work stays off the terminal thread.
6. Child processes are continuously drained, timed out, killed, and reaped.
   Reaping is enforced by the kernel, not by destructors: every child inherits a
   job object that terminates it when the court exits, including on a crash or a
   closed console window, where destructors never run.

## Ownership and status

| Folder | Owns | Migration state |
| --- | --- | --- |
| `Teledra/Brain` | Fixed-model Ollama boundary | Implemented; bounded response and CJK cleanup |
| `Agents` | Role profiles | Implemented as independent Markdown files |
| `Court` | Sessions and typed court actions | Implemented |
| `Teledra` | Wide/vertical TUIs and event coordination | Implemented; V1-wide and dedicated 54x48 portrait live proofs pass |
| `TTS` | Warm synthesis and playback | Implemented; CUDA worker warm-start reaches READY |
| `Music` | Score/render/playback/feedback and Organist DAW | Implemented; persisted score restores and verified synth/DAW start at boot |
| `Fractus` | Static/live verified visuals and animated gallery | Implemented; one supervised Studio hot-reloads PNG/GIF scenes |
| `Memory` | Recall and dream consolidation | Implemented natively |
| `Research` | Public research and provenance | Implemented natively |
| `Sidecars` | Optional external/device bridges | Implemented as supervised adapters |
| `Swarm` | Durable missions/tasks | Implemented natively |
| `Frankenswarm` | Court-panel composition plus immutable model locking, donor autopsy, merge tournament, evaluation, local server plan, SQLite blackboard/runtime, and sandbox | Implemented; current OLMo campaign is correctly blocked by a real token-ID incompatibility and unprovisioned heavy tooling |
| `Kraken` | Workspace verification plus standalone queue, skills, daemon, Hub workers, transactional forge, research/productivity, PixelFork, and behavioral game graduation | Implemented; legacy production inventory browser result reproduced live |
| `Tower` | Resident report inbox | Implemented natively |

## Deliberately rejected legacy patterns

- A monolithic application file owning UI, prompts, persistence, tools, audio,
  and processes. `main.rs` coordinates; feature changes live under
  `Teledra/src/runtime/` or their own crates.
- Model selection, remote chat providers, or API-key routing.
- Imports of executable code or mutable runtime state from `D:\Teledra`.
  Read-only fallback to the operator's existing Moltbook, outreach, MCP, and
  Restream credential files is supported until an equivalent V2 config exists.
- Model-authored arbitrary paths, shell commands, or automatic external posts.
- Unbounded response bodies, transcripts, PCM, image dimensions, task retries,
  process output, or conversation histories.
- Runtime-generated source mixed with maintained source.
- Copying V1 caches, databases, binaries, or abandoned experiment output.

`C:\Users\Kaged\Desktop\Teledra V2.lnk` launches the V2 release independently.
The V1 tree and its shortcut remain untouched as a rollback until the full OBS
live gate in `coordination/PARITY.md` has passed.

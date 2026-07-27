# Parity gate

The desktop shortcut **`Teledra v2 (qwen2).lnk`** defines what V2 has to
replace. It resolves to:

```
target      D:\Teledra\Teledra_v2_qwen.bat
working dir D:\Teledra
```

which sets `TELEDRA_CONFIG=config.qwen.json`, runs a release-freshness check,
and starts `target\release\teledra.exe`.

Everything that shortcut reaches is in the rewrite. Nothing in `D:\Teledra` is
deleted until the operator has confirmed V2 does the job; V1 stays runnable as
the reference and the fallback.

## Scope

Agreed: **core court, TTS, Music, Fractus.** Out: Kraken, Swarm,
Frankenswarm, Tower.

## In scope, landed

| Capability | Where | Notes |
| --- | --- | --- |
| Qwen-only brain | `Teledra/Brain` | Model is a fixed invariant, not config |
| Court identities | `Agents` | |
| Role-local sessions | `Court` | Separate history per member |
| Native TUI | `Teledra/src/ui.rs` | |
| Court portrait | `Teledra/src/portrait.rs` | Embedded; no disk lookup to break |
| Speech queue + warm voice | `TTS` | Removes V1's ~12 s/line dead air |

## In scope, remaining

| Capability | V1 reference | Blocking the gate? |
| --- | --- | --- |
| Audio playback + device choice | `src/voice.rs` | **Yes** — TTS reaches no speaker yet |
| Voice reference recordings | `assets/*_ref_clean.wav` | **Yes** — untracked in V1 git; must be copied from disk |
| Court speaks its own turns | `src/main.rs` | **Yes** — queue is not wired to the court |
| Music: score, render, feedback | `court_synth/`, `court_synthesizer.py` | Yes |
| Fractus: scene DSL, render, verify | `Fractus/` | Yes |
| Autonomous court cadence | `src/main.rs` | Yes — without it the court only answers when spoken to |

## Needs a ruling before the gate can close

"Core court" does not obviously settle these. They are reachable from the
shortcut today; each is in or out, and the answer changes the remaining work.

| Capability | V1 reference | Question |
| --- | --- | --- |
| Stream chat ingestion | `restream_listener.py` | The audience's only way to talk to the court. In? |
| Memory + dream consolidation | `retrieve_memory.py`, `dream.py` | Does the court remember across sessions? |
| Research | `browser_agent.py`, `get_youtube_transcript.py` | Drives the Scribe and study cycles |
| Outreach / Diplomacy | `outreach_poster.py` | Diplomat's actual effect |
| Treasury | `treasury_scout.py` | Treasurer's actual effect |
| MCP embassies | `mcp_bridge.py` | |
| Mic / vision / somatic | `copilot_mic.py`, `copilot_vision.py`, `somatic_cortex_stream.py` | Co-pilot mode; likely out |

## Confirming it works

The gate is a live run, not a green test suite. In one sitting, from the
shortcut repointed at V2:

1. The court starts, shows its portrait, and answers a typed question.
2. It speaks that answer aloud, in the right voice.
3. Several turns run back to back with **no audible gap** between them —
   this is the measured regression V2 exists to fix, so it is checked by ear.
4. Music plays and can be changed.
5. A Fractus scene renders.
6. Closing the court leaves no orphaned worker processes.

## Launching

Two shortcuts, deliberately:

| Shortcut | Runs | Role |
| --- | --- | --- |
| `Teledra v2 (qwen2).lnk` | `D:TeledraTeledra_v2_qwen.bat` | The working court. Untouched. |
| `Teledra V2 (rewrite).lnk` | `D:Teledra V2Teledra.bat` | This rewrite. For testing. |

The names collide badly: "v2" in the older one means the qwen2 model, not this
rewrite. Worth renaming the original to something like `Teledra (live).lnk`
once the gate passes and the confusion stops being useful.

The original is not repointed until the checklist above passes. It is the
fallback, and repointing it early would mean losing a working court mid-stream
to gain one that cannot yet play audio.

## Cleaning up, afterwards

Only once the above passes:

- Repoint the shortcut to `D:\Teledra V2\Teledra.bat`.
- Keep `D:\Teledra` intact but dormant for one full stream as a fallback.
- Then remove V1 surface, subsystem by subsystem, never faster than V2
  replaces it.

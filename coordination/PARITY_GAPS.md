# V1 to V2 parity audit

This audit is against `Xaiando/Teledra` commit
`5b57e7310ad42abc2425daa4cfff27d0788c0816`, plus the installed V1 operator
behaviour used on this machine. It records functional parity, not line-count
parity: V2 deliberately replaces the monolithic runtime with bounded owners.

## What "parity" means

The replacement gate covers the operator-facing sovereign court launched from
the repository root: its TUI, commands, court behavior, durable state, media,
device inputs, and configured external adapters. Generated renders, caches,
archived experiments, third-party checkouts, and old binaries are data, not
capabilities, and are not copied into V2.

Named standalone products under V1 are tracked separately. A crate with the
same name is not proof of parity, so Kraken and FrankenSwarm were audited and
rebuilt as standalone products rather than counted from their root-court helper
APIs. Their code surfaces are now migrated; remaining entries below are live
acceptance or external provisioning facts.

## Open parity tracks

| Surface | Current truth | Replacement condition |
| --- | --- | --- |
| Root court command heads | All 77 V1 command heads route in a pinned executable test | Preserve semantic tests as command behavior is migrated |
| Root court command behavior | Audited and restored; reflection, graceful `/Sleep`, direct-URL research, kingdom goals, bare mission status, mission-evidence binding, Treasury read-vs-scout, independent Night Desk toggle, diplomacy inspection, envoy speech, and outreach-log behavior are pinned by the 77-head compatibility suite | Preserve the executable command contract; only operator-visible hardware/external-service acceptance remains |
| Court autonomy | Public and Night Desk rotas now cover all 11 offices with independent counters | Confirm variety and pacing in one live transcript replay |
| Kraken standalone product | Queue, 12 typed skills, daemon/workers, Hub mission/task bridge, transactional repair forge, research/productivity, PixelFork, and behavioral game graduation are implemented. A read-only run against the V1 production inventory reproduced 15/16 browser graduations; `duchmans_mine` remains the same rejected non-contract artifact. | Visually accept the optional Hub desktop workflow if it will be used on air |
| Frankenswarm standalone product | Model locking/download manifests, semantic autopsy, deterministic tournament, evaluation/winner gate, local serve plan, SQLite blackboard/API, and networkless sandbox are implemented. Live metadata correctly rejects the configured phase-0 donors because vocabulary ID 100256 differs from the base. | Choose compatible locked donors or explicitly perform tokenizer alignment, then provision downloader/mergekit/Podman and run heavyweight tensor/merge/eval gates |
| Dedicated Dashboard and Work Board windows | Restored as simultaneous snapshot-driven read-only consoles launched by `/Dashboard` and `/Work` | Visually accept dimensions/font in the operator's Windows Terminal/OBS setup |
| Hardware and external services | Code paths exist; acceptance remains environment-dependent | Complete the live-machine gates below |

## Restored functional surface

| V1 behaviour | V2 implementation |
| --- | --- |
| Queen identity, physical canon, court relationships, lore/fourth-wall rules | Substantial per-role files in `Agents/profiles` |
| Writer, hard-rule critic, minimal refiner | `Teledra/Brain` reviewed completion path |
| V1 hidden action vocabulary and space-delimited delegation | Typed parser in `Court/src/actions.rs` and runtime owners |
| Taste, opinions, curiosity, recurring desires and decay | Structured bounded state in `Memory` |
| Audience return memory, conduct and Sovereign Tokens | Audience and token ledgers in `Memory` |
| Wide V1 control-room layout and dual inputs | `Teledra/src/ui.rs`; V1 half-block portrait is the default |
| Multiline paste | Bracketed-paste support in `Teledra/src/app.rs` and terminal setup |
| Simultaneous OBS portrait view | `/Vertical`, shared snapshot, dedicated 54x48 console |
| Restream embed-link connection and saved-token startup | Supervised persistent `restream_listener.py`; token enters through stdin |
| Continuous co-pilot microphone toggle | Ctrl+J polling with local faster-whisper; `/Mic` remains one-shot |
| Co-pilot vision | Screen capture, local moondream description and OCR fallback |
| Somatic state | OpenCV/MediaPipe camera bridge and live TUI/prompt telemetry |
| Court music, composer nudge, DAW transport and feedback collections | Startup-restored native score/harmony engine, verified single-source playback, hot-reload Organist Studio, Ctrl+U evolution, keepers/playlist |
| Fractus app, animation, scene catalogue and layered blending | Supervised hot-reload Fractus Studio, distinct-frame GIF animation, living PNG presentation, 36 canonical families (plus V1 aliases), palettes, bounded layers and blend modes |
| Research evidence and durable facts | Public-source provenance and exact-clause verification |
| Workshop-authored tools | Capability scan, sandbox smoke test and explicit promotion |
| Night Desk study/creation/diplomacy/treasury cadence | Role-aware backstage schedule and audible treasury update |
| Treasury scouting | Public-feed opportunity queue with SSRF protection and no auto-application |
| Moltbook posts, replies, upvotes and inbox | Current API client, cooldowns, credential-host lock and `/Approve` gate |
| Wizard resident reports | Flexible JSONL import, deduplication and periodic polling |
| Warm voice worker and role voices | `TTS` worker with cached LuxTTS/Whisper models |

## Deliberate V2 boundaries

- Model-authored Python music and Strudel are retained as reviewable artifacts,
  but are never executed automatically. The native score engine is the live
  execution path.
- External writes remain operator-gated. A model proposal cannot post, call an
  MCP tool, or promote a workshop experiment until `/Approve`.
- Moltbook verification challenges are reported backstage and are not solved
  automatically because that would exceed approval for the original action.
- Existing V1 credentials are read only as a migration fallback. A V2 config
  in the data directory always takes precedence.

## Live-machine gates

No root-court or standalone-product code owner is missing against the linked
source. The remaining checks depend on hardware, external services, model
provisioning, or operator-visible layout and cannot be proven by unit tests:

1. Camera index 0 currently reports unavailable; somatic fallback is healthy,
   but live pose tracking needs a connected camera.
2. OBS capture, selected audio routing and the nine role voices need one live
   operator session.
3. The saved Restream token completed a websocket connection and authenticated
   Moltbook `/home` returned HTTP 200 on 2026-08-02. Outward posts/comments still
   need operator-approved live use; V2 does not emit a test post merely to prove
   write access.
4. The dedicated portrait console should be visually confirmed in the chosen
   OBS font and scene before the first broadcast.

## Live-operation defects found by running the court

These were not parity gaps — every owner existed and every test passed. They
were found by watching the running court and reading what it actually produced.

| Defect | Effect | Fix |
| --- | --- | --- |
| Action tags written without brackets (`**COURT_SCORE: {...}`) | Every composition discarded; the score never changed | Bare/markdown tag recovery in `Court/src/actions.rs` |
| Scribe body written after the closing bracket | All scribe records failed | Trailing-body capture, terminating on an indented tag |
| `FRAXUTS_ART` and other misspelled heads | No art ever rendered | Bounded edit-distance correction, outward heads excluded |
| Deadline spent on a refused turn | Night Desk starved the public court into silence | Deadline advances only when a turn starts |
| Unchanged Moltbook digest re-sent every 60s | The same upvote proposed 24 times | Digest fingerprinting |
| Identical pending proposals | 113 pending, 34 distinct | Dedupe on write, collapse on startup |
| Fixed idle prompt | One line spoken nine times in a row | Rotating angles, opener named back, duplicate suppression |
| `Voice::level` clamped after gain | Every emphatic syllable hard-clipped | Soft knee above 0.62 |
| Queen voice modes absent | TUI reported "custom"; worker could not produce it | V1 references restored, mode resolved in one place |
| One invalid layer rejected a whole Fractus stack | Every artwork fell back to identical default particles | Valid layers kept, unknown ones recorded |
| DNS failure reported as a policy refusal | Misleading research errors | Distinct `Unresolvable` variant |
| `Teledra.bat` built only when the exe was missing | Source changes never reached the running program | Launcher always builds |
| Unvalidated `palette` field swallowed layer stacks and prose | Every artwork rendered as the identical default scene | Palettes are a validated set; each payload shape is recognised |
| Layer stack never set the base scene | A kaleidoscope request still drew particles underneath | The first layer becomes the subject |
| Score changed only when the Organist happened to write one | One loop could hold an entire stream | Native verified evolution every piece (20--45 seconds by default), plus the slower model-composer nudge |
| Keep/reject buttons wrote write-only feedback | The court could not tell a kept piece from a refused one | Verdicts are quoted back in the composer nudge |
| Destructors skipped on abnormal exit | Six Python children orphaned, holding the mic and audio device | Job object binds every child to the process lifetime |

Two things a replay harness makes cheap, and both are worth keeping:
`cargo run -p teledra-court --example replay_transcript -- <transcript>` shows
what fraction of the court's requests actually execute, and
`cargo run --release -p teledra-music --example arrangement_report` shows what
fraction of written notes are sounded.

### Why a correction loop, and not more parsing

Each recovery above fixed the shape the model was producing at the time, and
the model then produced a new one: brackets dropped, then names misspelled,
then a score written as markdown headings. Recovery is reactive by
construction — it can only ever handle shapes that have already been seen on
air.

The runtime now hands the parse error back instead. A `COURT_SCORE` or
`FRACTUS_ART` payload that does not parse produces one backstage correction
request quoting the error and the offending text, bounded to a single attempt
per distinct payload so a stubborn reply is dropped rather than looping. That
covers shapes nobody has anticipated, which the recovery paths cannot.

Keep the recovery paths: they cost one parse and they spare a whole inference
for the mistakes that are already known. The loop is what catches the rest.

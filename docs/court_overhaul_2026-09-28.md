# Teledra's Court overhaul ledger — 2026-09-28

Baseline repository commit: `c8454f2447e6ee4812197a09f9937ca34258cd4c` (`main`).

Working branch: `court-overhaul-audit-2026-09-28`.

This ledger is the durable handoff for the audit → reproduce → implement → critique → debug → verify cycle. It deliberately distinguishes repository-verifiable work from checks that require the operator's local models, voices, recordings, GPU, or current uncommitted Court.

## Operating constraints

- Preserve the existing Rust TUI, Brain/provider configuration, VoiceEngine, Court Synth, and one-controller Court Radio architecture.
- Do not redesign Buzzart/Organist unless a reproducible routing, score, handoff, clipping, or playback fault appears.
- Synthetic tests must not write to live character memory.
- No provider/billing changes, publishing, livestreaming, external posting, permission weakening, or user-data deletion.
- Stale generation must be discarded after an operator turn/topic replacement.
- Transcript evidence is speech evidence only; it does not license invented visual observations.

## Baseline findings

| Concern | Reproduction / diagnosis | Repository change | Acceptance evidence | Local follow-up |
| --- | --- | --- | --- | --- |
| Teledra entertainment value | The active Queen prompt is already intentionally eccentric and contains strong anti-generic anchors. No evidence in the repo supports replacing it wholesale. The higher risk is regression from downstream shortening/refiner/runtime differences in the newer local tree. | Preserve the existing persona architecture. Deep media commentary now uses Streamer cadence rather than a tiny Normal reaction and carries continuity between source segments. | Existing Queen prompt contains explicit aliveness/style constraints; CI/unit tests protect changed paths. | Run matched old-good vs current-local prompts and save quoted samples for human aesthetic judgment. Check local critic/refiner changes not present in this July repo. |
| YouTube commentary shallow/short | Clean repo called missing `get_youtube_transcript.py`; no transcript dependency was declared. Runtime truncated transcript to 4,000 characters and made exactly one ordinary Queen call. This guarantees shallow source coverage even when the model is capable of more. | Add timestamped transcript sidecar + dependency. Replace 4k one-shot path with bounded multi-segment deep commentary, continuity tail, real source-position labels, stale-turn cancellation, and a configurable 1–6 segment target (default 5). Brain prompt explicitly forbids invented visuals and transcript-embedded instructions. | Unit tests cover contiguous short-source windows and long-source sampling through the end. Windows CI installs the new dependency, compile-checks Python, builds Rust and runs tests. | Real-source smoke checks: substantial explanatory video, short amusing clip, unavailable/private/no-transcript source. Listen to delivered TTS; generated text alone is not enough. |
| Radio-host path | No independent “radio host node” exists in this repository. The real end-to-end implementation is `/lock` → `BroadcastSession` → `BroadcastTicket` → Brain Broadcast turn → validation → session-tokened speech queue → VoiceEngine. It already enforces one broadcast session and tokened foreground speech. | Preserve the existing controller instead of adding a parallel node. Topic replacement/unlock now also invalidate in-flight turns and stop active playback so old speech cannot leak across the boundary. | Existing tests cover tokened foreground admission, counterpoint routing, exact handoff continuity, action-tag stripping, and locked-topic prompt anchoring. CI exercises the compiled path. | Run actual start → host → counterpoint → synthesis → music bridge → pause/interruption/stop using local TTS/assets. “Pause/resume” is not a first-class Court Radio command in this repo and remains a local/product gap. |
| Topic locking | Lock state is split between compatibility `locked_topic` and authoritative `BroadcastSession`, but generation/speech checks key off the session id. The main concrete leak found was replacing/unlocking a session without stopping an already-playing old broadcast. | `/lock <new topic>` and `/unlock` now call `begin_user_turn()`, remove session-tokened queued speech and drop the active PlaybackController (Drop cancels TTS). Old async BroadcastReply already fails the session-id check. | Existing stale/session-id tests plus the new hard-boundary implementation. | Deliberate off-topic chat, transcript injection, slow old task arriving after topic switch, restart/persistence behavior. Persistence is not promised by this repo; lock is process-lifetime state. |
| Buzzart / Organist | Organist has extensive score validation, no-op detection, recovery, feedback, and Court Synth tests. No repository evidence justifies an engine rewrite. “Buzzart” is not a named runtime role in this July tree; likely local-only/newer work. | No music-engine redesign. Radio MusicBridge explicitly keeps Organist in verbal music-director mode and forbids an unsolicited new CourtScore. | Existing Organist/Court Synth tests remain untouched and must pass. | Compare local Buzzart/Organist against this branch and cherry-pick only regression protection / routing fixes that survive both test suites. |

## YouTube implementation details

The default target is five commentary segments, bounded by `TELEDRA_YOUTUBE_COMMENTARY_SEGMENTS=1..6`. Each segment receives a bounded transcript window and the tail of the previous commentary. Short sources naturally produce fewer windows; thin material is explicitly allowed to stay short.

The transcript sidecar emits timestamp prefixes such as `[03:17]` when the upstream transcript exposes start times. Deep commentary prompts label character ranges as a second provenance mechanism. No frame extraction is claimed here.

Cancellation is operator-turn based: every segment checks the active turn epoch before and after inference. A new operator turn, lock replacement, or unlock makes old work stale.

## Deterministic scenario matrix

1. **Free Court banter** — no `BroadcastSession`; ordinary speech uses un-tokened foreground.
2. **Locked discussion** — `/lock <topic>`; only speech with the active session id is admitted.
3. **Lock replacement** — `/lock <other topic>`; old generation epoch invalidated, old active playback dropped, queued tokened speech removed, old BroadcastReply rejected by session id.
4. **Explicit unlock** — same hard cancellation boundary, then ordinary speech is allowed again.
5. **Deep commentary / rich source** — timestamped transcript → 2–6 source windows → sequential Streamer-style commentary → one bounded delivered response.
6. **Short clip** — naturally one/few windows; prompt forbids padding.
7. **Unavailable source** — sidecar exits non-zero; runtime surfaces ingestion failure, no invented commentary.
8. **Interruption during deep commentary** — new user turn changes epoch; next segment aborts as stale.
9. **Court Radio stale result** — old session id result is ignored after switch.
10. **Music transition** — Organist MusicBridge speaks over the existing keeper but cannot emit/replace CourtScore in Broadcast mode.
11. **Source/tool failure** — Broadcast retry is bounded; YouTube fetch failure is terminal and explicit.
12. **Rumor/provenance correction** — Scribe FactCheck uses only verified source cards or states uncertainty.

## Commands for a clean checkout

```powershell
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install --upgrade pip
pip install -r requirements-all.txt
python -m compileall -q -x "\.venv|tools[\\/]experiments|kraken[\\/]workspace" .
python -m unittest discover tests
cargo fmt --all -- --check
cargo build --release --locked
cargo test --all-targets --locked
cargo run --release --locked -- --minimal --check-environment
```

Runtime:

```powershell
$env:TELEDRA_CONFIG = "config.qwen.json"
# Optional: 1 = brief, 5 = default deep session, 6 = upper bound
$env:TELEDRA_YOUTUBE_COMMENTARY_SEGMENTS = "5"
cargo run --release -- --minimal
```

## Local real-time acceptance

The GitHub repository contains no representative Court recordings and cannot expose the operator's local Ollama models, LuxTTS voice assets, current Court memory, or newer uncommitted/local-only roles. Therefore the following must be run on the local merged candidate and must not be mislabeled as repository-verified:

- 20–30 minute real-time Court soak with at least one `/lock` replacement and `/unlock`.
- Rich-video commentary listened to end-to-end; record wall-clock, active speech duration, silence gaps, clipping, repeated intro, stale speech, and source-specificity.
- Short funny clip and unavailable-source checks.
- Buzzart and Organist independent runs plus one Court transition each.
- Operator interruption while Teledra is speaking and while a deep-commentary segment is generating.
- Compare earlier-good Teledra samples against the merged candidate for specificity, varied reactions, contextual eccentricity, callbacks, quiet sincerity, and coherence. Do not grade “gothic word count.”

## Resumption note for Codex / local integrator

Start from this branch and compare it against the current local tree rather than force-merging the July architecture wholesale.

Priority merge order:

1. Keep whichever local transcript/media extractor has richer evidence (title/description/timestamps/frames/audio) **if** it preserves the source-grounding rules and stale-turn cancellation added here.
2. Preserve the single authoritative local show controller. Port the hard topic-boundary behavior: new lock/unlock must cancel old generation + queued speech + active playback.
3. If the local Teledra persona has regressed, diff the active runtime prompt/refiner/sanitizer/speech limits against `src/brain.rs` and `QUEEN_VOICE_ANCHOR`; do not blindly copy prompt prose.
4. Keep local Buzzart/Organist improvements unless deterministic routing/audio tests fail. Do not replace their engines merely to match this branch.
5. Run the full deterministic suite, then the real-time audio soak above. Preserve representative failures and before/after transcripts for final human creative judgment.

Unresolved repo-level limitations:

- Court Radio has stop and interruption behavior but no explicit pause/resume command/state machine in this July tree.
- YouTube extraction is transcript-only; it does not fetch frames/title/description in this branch.
- No checked-in audio fixtures allow objective listening validation in GitHub CI.
- Lock persistence across process restart is not implemented or promised in this tree.

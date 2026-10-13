# Recursive music and art development — 2026-09-13

The operator prefers the sound of PyMusic and requested fluid ongoing development
instead of getting stuck on one track. They explicitly chose **PyMusic evolution
with Native Synth available for manual comparison**.

## Causes verified

- The only accepted Python render was the 88-second V1 Fivefold Court Engine.
  With `alternating=true`, subsequent composition requests targeted Native. A
  failed Native attempt left Python selected, so another cycle targeted Native
  again without ever revising Python.
- A timed-out composition incorrectly triggered a missing-score prose correction.
- Generation normally started only after two complete passes (at least 120s),
  then waited for another complete playback boundary. A second independent
  five-minute timer could lose requests while public speech was overdue.
- Python prompts used an editable draft and the native score's key/tempo. The
  accepted Python piece actually uses A harmonic minor and 109.09 BPM.
- Accepted art progress reset on restart, simple drift repeated four zoom/motion
  settings, and selected-but-unrendered scenes replaced the accepted parent.

## Implemented behavior

- Actual saved playback is Python with alternation disabled. Native comparison
  remains available manually. Direct operator guidance is persisted separately
  from artifact ratings in `music/learning.json`.
- A single runtime cadence prepares development during current playback, normally
  at 55% of the piece, bounded 30–120s. Busy composer admission does not spend an
  overdue deadline. Public/background fairness also works with a sustained
  audience queue. Complete replacements still wait for a full musical boundary.
- `PYTHON_MUSIC_EDIT` applies 1–8 exact, nonoverlapping changes against a stable
  accepted-source fingerprint. All changes validate before application; stale,
  ambiguous, empty, oversized or unchanged edits fail without changing playback.
  Full Python compositions remain supported for new pieces.
- Every edited result goes through the actual complete Python render and audio
  verifier. Exact source/PCM repeats from generated work do not become new
  accepted revisions. Restore and explicit editor Run remain available.
- Accepted source/report/key/tempo are distinct from drafts. Historical feedback
  is projected to bounded evidence rather than including whole old source files.
  Music learning retains 32 outcomes plus progress and accepted parent identities.
  Acceptance means technical validity, never a fabricated listener verdict.
- Playback replacements reuse one output stream/sink with 40 ms smooth overlap.
  Incoming samples are consumed in order; pause/mute and mixer position persist.
  The finished-one-shot replacement race is covered by a deterministic test.
- Model/render/acceptance failures retain prior music and re-arm development.
  A model error does not trigger the misleading missing-score correction. Missing
  Python output retries Python, including when comparison is enabled.
- Art keeps 32 bounded outcomes linking accepted parent, canonical DSL, measured
  render facts or exact errors. Progress survives restart. Routine development
  asks the Artist for one purposeful revision; exploration remains available.
  Only a current successful render promotes staged lineage. Failed/stale work
  cannot replace the parent or disable newer research.

## Verification

- Final release workspace suite: 559 passed, 0 failed, 15 optional tests ignored.
- Strict Clippy for Teledra/Music/Fractus/Court all targets and diff whitespace
  checks passed. Release build and read-only environment check passed.
- Explicit muted device/window lifecycle proof passed: exclusive switching,
  unchanged-generation rejection, persistence-failure recovery and manual restore.
- Off-device recursive proof uses the real Court parser, edit utility, Python
  worker and VerifiedAudio: original V1 plus two successive focused revisions.
  All three are 88s, 754/754 completed helper calls, 682 intentional declared events,
  eight active layers and five active sections. All three PCM outputs differ;
  the seven untouched layers remain byte-identical. The stale first baseline is
  rejected for the second revision. Report:
  `target/recursive-python-proof/1789325861203/proof.json`.
- The 754 helper calls and 682 events are deliberately different: V1 generates 72
  unused kick buffers. Helper completion, declared events and observed layer
  activity are separate evidence; their equality is not a correctness gate.
- Art tests cover actual 120-frame rendering, accepted-parent promotion,
  failure/stale-result preservation, corrupt-journal recovery and persisted
  context/progress. Independent critics reviewed the changes.

## Activation and limits

`target/release/teledra.exe` is rebuilt. The running court was left on its previous
executable, renamed to `teledra-before-recursive-music-20260913.exe`. Restart through
the usual launcher to load the new pipeline and saved PyMusic-only preference.
The original comparison setting is backed up in the application music directory.

The proofs validate editing, rendering, scheduling and transport. They do not
prove that the live model will choose aesthetically better changes. Improvement
here means carrying accepted work, measured outcomes and genuine operator
feedback forward; no model weights are retrained. A single model still performs
inference sequentially, and a composition may use its 90s total budget.

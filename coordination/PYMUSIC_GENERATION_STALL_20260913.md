# PyMusic generation stall — 2026-09-13

The operator reported that only the original track was still playing after the
recursive pipeline changes. The updated executable was running: this was not a
stale-build explanation. The live journal recorded nine attempts, but none
reached a new Python render. The accepted artifact remained the 88-second
Fivefold Court Engine, with 754/754 completed synthesis helper calls.

## Confirmed causes

- Python artifacts used the ordinary court dialogue/review path. Its refiner
  prohibited JSON, lost dispatch-specific source context, and did not protect
  the `PYTHON_MUSIC_EDIT` tag from replacement with prose.
- Historical request examples reentered the prompt with placeholder source
  identities. The three stale-source rejections occurred while the actual
  accepted baseline had never changed.
- Freeform model proposals remained unreliable after structured JSON was added.
  The first real trial produced a no-op; the second invented an old source line
  and repeated it after the actual validation error. Numbered source addressing
  removed that copying requirement, but the third trial produced incomplete
  Python fragments and incorrect indentation. Rejecting these was correct.
- Raw Python embedded directly in an action tag could be truncated by Python
  single-quoted brackets or quote-bearing comments. New source now travels as
  a JSON envelope through the parser, which extracts the exact code.

## Implementation

- Executable composition has a fresh artifact-only conversation, bounded recent
  outcomes, operator guidance, actual accepted facts and all 72 instrument IDs
  with their comfortable ranges. It bypasses the dialogue critic/refiner.
- Rust supplies the accepted source identity; the model never invents it. The
  original version, overlap, no-op and size checks remain enforced.
- The restored V1 arrangement exposes typed musical timing controls. Rust
  updates the corresponding working source and note-event declarations together.
  This supports timing/articulation development while preserving existing pitch
  material; it is not evidence of general reliable Python programming.
- Other freeform sources retain numbered-line editing, with original indentation
  and CRLF/LF endings preserved. Full-source generation and manual edits remain
  available. All resulting code still passes through the complete Python worker
  and audio verifier before the one-player boundary handoff.
- A generation and its one factual repair share the same total deadline. The
  optional `TELEDRA_PYTHON_TRACE_DIR` records bounded raw requests, responses and
  validation outcomes without affecting acceptance.
- Session snapshots now expose cached source/PCM identities, render/ready state,
  playback timing and the next evolution deadline for operational diagnosis.

## Real-model evidence

The opt-in integration harness uses the production context, generator, action
parser, edit utility, real Python worker and audio verifier. It reads the actual
accepted source without changing AppData or opening an audio device. Each attempt
is bounded; there are no hand-authored replacement notes or source fixes.

The initial failures are retained as evidence:

- `target/live-python-evolution-proof/1789327630917`: no-op edit.
- `target/live-python-evolution-proof/1789327912294`: invented old source line;
  complete raw requests/responses under `raw-second-trial`.
- `target/live-python-evolution-proof/1789328234469`: incomplete Python fragments;
  complete raw requests/responses under `raw-third-trial`.

The fourth trial produced two complete revisions through typed controls. Its
critic found that the second revision changed phrase spacing despite a request
to preserve the lead. The runtime now removes lead-gate and phrase-breath controls
from both the offered schema and accepted changes for that request.

The final fifth trial passed with that restriction:

- `target/live-python-evolution-proof/1789329897238/proof.json` and
  `target/live-python-evolution-proof/raw-fifth-trial/` retain the full evidence.
- Original and both model-authored revisions render for 88 seconds, retaining
  the key, tempo, source sample rate, all eight active layers and five sections.
- Requested/completed note-helper counts are 754/754, 740/740 and 738/738. Lower
  counts reflect model-selected phrase rests and sparser answers, not missing
  synthesis calls. Declared events and observed layer audio are checked separately.
- The first answer moves from beat 69.25 to 69.5 to 69.75; its recorded duration
  changes from 0.7 to 0.9 to 1 beat. Every final PCM differs. Six layer arrays stay
  byte-identical in the first revision. In the second, only the counterline
  changes: all seven other layers and every lead event remain identical.
- Model values were not substituted or repaired by hand. The engine only
  materialized the validated controls and coupled audio/event changes.
- This proves working phrasing/timing variation for the recognized V1 source,
  not an aesthetic verdict or general reliable Python rewriting.

## Final verification and activation

- `cargo test --workspace --release --quiet`: 613 passed, 0 failed, 16 optional
  tests ignored. Log: `target/pymusic-generation-workspace-tests.log`.
- Strict Clippy for Teledra, Music and Court, all targets, passed. Formatting and
  Git whitespace checks passed.
- `cargo build --release -p teledra` and `teledra.exe --check` passed. The normal
  release executable is rebuilt (2026-09-13 22:06 local, 13,585,920 bytes).
- No running court was left at final verification. Start the usual `Teledra.bat`
  to load this build. Saved selection remains Python, alternation disabled.
- Proofs did not modify accepted AppData music or start playback. The runtime
  will prepare the next variation during playback and publish it at a complete
  piece boundary through the existing single-player handoff.

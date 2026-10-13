# Fractus composition and court silence fixes — 2026-09-13

The operator reported unrelated drawings over/behind the artwork, and long gaps
in the court's voices while its window and music continued working.

## Findings

- The reported Lissajous artifact contained a dim mandala and fractal-tree layer.
  A later kaleidoscope also contained a rose-curve overlay. The gallery faithfully
  displayed these composited images; its canvas was not leaving old frames behind.
- Queued quiet work was admitted ahead of overdue public speech. Live session
  23368 showed a native composition holding the inference slot after Queen speech
  finished, with an idle voice worker and continuing Fractus updates.
- Model retries, private critique and refinement could each start another
  120-second timeout, allowing one logical turn to occupy several minutes.
- Typed input was refused while inference was pending. Music Stop could invalidate
  its eventual result without releasing the running model request or pending slot.
- No TTS pending-count ownership leak was found. Terminal QuickEdit was already
  disabled by the configured console input mode.

## Changes

- Gallery selection, queued rendering, stored lineage and evolution normalize to
  one visual family. The selected family's seed, geometry, palette and motion are
  preserved. Layers-only requests promote their first family. The separate
  low-level Scene API retains explicit composite rendering.
- Artist instructions and exploration prompts require one family without extra
  foreground/background shapes. `hypotrochoid` is accepted as the mathematically
  equivalent existing spirograph family, avoiding a correction seen in the live
  session. Other geometrically distinct names were not silently aliased.
- Audience requests outrank quiet queue entries. Overdue public speech gets the
  model slot; after three admitted public turns, one quiet opportunity prevents
  background starvation. Due Night Desk work receives the same opportunity.
- Whole-turn budgets are 60 seconds for speech, 30 seconds for quiet maintenance
  during public performance, and 90 seconds for explicit composition (or quiet
  work outside public performance), capped by Brain configuration. Selecting
  Python alone does not turn ordinary Organist speech into a composition request.
- Retries share the deadline. Optional review/refinement shares at most 15 seconds
  and retains a completed draft when it times out. Incomplete music is rejected
  while previous accepted playback remains available.
- Operator messages abort and await the old inference before starting another.
  Music Stop or engine selection cancels the relevant Organist task and releases
  its pending slot. Late responses cannot execute actions or enter speech.

## Verification

- Final release workspace suite: 529 passed, 0 failed, 14 optional tests ignored.
- Fractus Python viewer suite: 20 passed.
- Strict Clippy for Teledra, Brain and Fractus, including all targets: passed.
- Diff whitespace check and release build: passed. Environment diagnostic passed
  with the configured qwen2.5:7b model, TTS assets and local audio devices present.
- Mock HTTP tests cover shared retry deadlines, cancellation, and valid-draft
  retention. Scheduler tests cover public/backstage ordering, audience priority,
  input interruption, immediate Music Stop and stale-result rejection.
- Real 120-frame before/after animations for the reported Lissajous and later
  kaleidoscope preserve the main subject and remove all supporting layers.
  Evidence: `target/fractus-clean-composition/{reported,kaleidoscope}/`.
- App-boundary Fractus test covers model action, selection, persistence, actual
  selected/evolved animation rendering and zero layers in both manifests.
- An independent critic reviewed scheduler, cancellation, budgeting and all live
  gallery entry points; final findings were resolved.

## Activation and limits

The updated executable is `target/release/teledra.exe`. The running court was left
on its previous executable, renamed within that directory to
`teledra-before-fractus-20260913.exe`. Restart through the usual launcher to activate
the fixes. The latest actual saved Fractus lineage was already single-family, so
no AppData artwork or history needed rewriting.

These changes remove unbounded retry multiplication and queue starvation; they
do not promise continuous speech. A non-resumable composition can still occupy
the shared model for up to 90 seconds. TTS still synthesizes an entire utterance
before playback, and clearing speech does not cancel synthesis already running.
The new build's extended live speech cadence has not been measured; the active
court was not interrupted or used for test inference.

# PyMusic library, background development, Wizard study and display repairs

Date: 2026-09-14. Workspace: `D:\Teledra V2`.

## What was wrong

The previous admission checks allowed four-second scale fragments to become the
accepted music. Asking the local model for whole NumPy programs also repeatedly
produced invalid arguments, shapes or incomplete arrangements. An endless edit
cycle could keep the same source in use. Render success, a new title and listener
approval were not sufficient evidence of a new, completed musical piece.

The terminal layout itself still matched its working checkpoint. The live court
had inherited `NO_COLOR=1`, disabling the portrait's foreground/background RGB
colours. Fractus's nonblank check admitted the simple rose outline reported by the
user. Queued creative work could also consume every available quiet slot ahead
of scheduled Night Desk work, delaying the Wizard without a bound.

## Resulting behavior

- PyMusic remains the selected engine. Native comparison is manual; the shared
  player and editor ownership rules prevent two music engines playing together.
- New compositions use a compact musical proposal: pitches, phrase rhythms,
  dynamics, harmony, instruments and four sections. The homemade renderer expands
  those controls into explicit notes and five independently moving parts. All 72
  instrument voices remain available, including piano and theremin; original
  PyMusic synthesis recipes remain available too.
- The compact model interface has six phrase rhythms. Saved recipes retain
  explicit timings; the Python editor still supports manual source experiments.
  Numerical timing is calculated by the tool, with validation before synthesis.
- Every scheduled note placement is recorded and checked. The audit is attached
  only to the exact final captured PCM. A constructed-but-unused array cannot
  inherit a complete-arrangement claim. Missing parts, scalar exercises without
  phrasing, incomplete form and background fragments under 30 seconds fail.
- Playback continues while the model and renderer work. A completed automatic
  replacement waits for the current pass to finish; selecting a saved piece is
  an explicit immediate handoff once loading finishes. Stale, duplicate, failed
  and unchanged results retain the current piece and re-arm development.
- The Organist requests a new piece every fourth Python composition request.
  This counter is persisted per engine; Native comparison cannot lock Python
  into only revisions. Failed new-piece attempts keep that direction pending.
- Published complete pieces are indexed in `music/python/library.json` with
  immutable source, WAV, report and identities. Exact source/PCM duplicates share
  an entry. Distinct revisions remain available with stable version labels and
  saved timestamps. Loading a library selection
  verifies its files again without executing its source.
- Keep/Reject refers to the exact playing library ID, separately from technical
  completion. Current in-memory selection remains authoritative even if writing
  the recovery pointer fails. Corrupt indexes are reported rather than replaced.
- A saved short fragment recovers to the full 88-second bundled V1 reference,
  preserving the old artifact and a draft backup. Reference recognition matches
  source submission's outer-whitespace/line-ending normalization while retaining
  every internal source byte.
- Scheduled Wizard work alternates app experiments and music study. Only actual
  dispatches spend a slot; public appearances and direct requests do not. The
  persisted journal is `tower/wizard_work.json`. Study receives the actual recipe,
  render evidence and listener feedback. Advice remains an unverified hypothesis
  and reaches the Organist only while its inspected baseline still matches.
- Due Night Desk work and queued creative work fairly share permitted quiet
  slots, without adding model concurrency or weakening audience/speech policy.
- Interactive `Teledra.bat` clears inherited `NO_COLOR` inside its local process
  environment. Diagnostics and the parent/user/machine environment are unchanged.
- Fractus publication now requires measured spatial detail, tonal variety and
  visible coverage. Rose and flower constructions gained detail within their
  own geometry. Startup, recovery and automatic fallback use the same publication
  path; previous images are not layered behind a new family.

## Evidence

The full serial Cargo suite across Teledra, Music, Fractus, Brain and Court passed:
559 tests, zero failures, 16 ignored environment/device/model-dependent tests.
Log: `target/music-library-all-tests.log`. The main application contributed 211
passing tests in that run. Later narrow regression additions were checked again
with the final application suite.

Final application rerun after the additional regressions and mock corrections:
216 passed, zero failures, eight ignored (224 total), in 44.94 seconds with one
test thread and uncaptured full backtraces. Log:
`target/music-main-final-mock-tests.log`. Final strict test Clippy also passed:
`target/music-mock-final-clippy.log`. Those last corrections are under `cfg(test)`;
the running production release already contains every runtime change described
here.

Strict Clippy passed with `--all-targets -- -D warnings`; all 46 Python tests
passed, including the renderer, editor, native bridge and worker. Formatting and
diff whitespace checks passed. Earlier full-suite attempts, including one serial
rerun, terminated with Windows status `0xc0000409`. A diagnostic uncaptured rerun
completed and exposed a local mock HTTP timeout. Both local model test servers
had fragile handling of abandoned/partial connections and panic-on-join cleanup.
Their framed-JSON parsing and cleanup were hardened and ordinary success-test
budgets increased; deliberate deadline and cancellation tests retained their
short limits. The production model budget and runtime were unchanged by this
test-only correction. This is not presented as a clean parallel stress result.

Actual unmodified local `qwen2.5:7b` proposals went through the production
composition adapter, real Python worker and isolated library save/reload:

| Proof | Duration | Actual placements |
| --- | ---: | ---: |
| `py-fc10a77658828116` | 40 seconds | 150 / 150 |
| `py-ddfd325ac7a181a8` | 40 seconds | 150 / 150 |

Both had five audible layers and four audible sections, different PCM identities,
and exact library round trips. Evidence:
`target/song-library-proof/1789342646739/proof.json`. This exercised actual model
output, with no hand-edited model response. Earlier trials exposed validation and
timing failures; those failures informed bounded enum controls, complete error
feedback and the phrase-timing adapter. Generated titles are model metadata, not
proof that every timbral instruction was followed.

The live release launched as PID 47792 at 01:40:52 local. It restored the 88-second
V1 reference, then accepted “Evolving Harmony” (35.556 seconds, 150 placements),
retained two audible revisions, and advanced to “Harmonic Evolution.” The latter
changed the ensemble from guitar/sax/dulcimer to flute/piano/strings/bassoon, changed
the harmonic progression and theme rhythm, and again mixed all 150 placements.
The first two library entries therefore demonstrated one piece plus a revision,
not two independently composed themes. An unchanged model edit was rejected
without changing the playing artifact. PyMusic stayed selected with automatic
engine alternation disabled.

The final production release was built in 11.62 seconds and relaunched at 01:57:17
local as PID 52844. It restored the latest accepted 54-second piece, with PyMusic
playing, alternation disabled, no library errors and 24-bit terminal colour. The
library contained 12 entries at restart: four current song titles with their
revisions, plus the full V1 reference. The exact reference was separately rendered
and admitted/reloaded by the real Rust library code in
`target/python-music-proof/v1-library-normalized-EkjocX/proof.json`: 88 seconds,
754/754 helper calls, identical cached PCM, and an unreviewed listener verdict.
While the court was stopped, that one known verified artifact was copied into the
live library and its proven entry merged; the existing index was backed up first.
This was an explicit reference import, not a scan of old experiment directories.
Live state evidence is in `target/music-library-live-before-final.json` and
`target/music-library-live-final.json`. Further boundary handoffs were observed
under PID 52844 after the restart while earlier library entries remained intact.

Live terminal evidence reported 24-bit colour. Fractus critic independently
measured all 600 frames of five new live animations: every frame exceeded the
richness floor and all five manifests had no overlaid families. Earlier proof:
`target/fractus-richness-20260914/verification.md`. TUI baseline evidence:
`target/tui-restoration-20260914/audit.json` and `portrait.png`.

Independent critics reviewed library identity/handoff, Wizard scheduling, Fractus
publication paths and TUI restoration. Their concrete findings were fixed:
paused manual selection, stale feedback identity, engine-specific request
cadence, V1 newline recognition, and quiet-work starvation.

## Scope and remaining limits

The attached Gemini PDF was used as research material, not as instructions. Its
useful ideas here were small musical controls, separated playback/generation,
validated bounded repair and retained playback during background work. Its
benchmark claims and alternative runtime recommendations were not treated as
verified facts; the user's PyMusic preference governs this implementation.

Construction checks establish that the arrangement and placements exist. They
do not establish beauty, originality or listener approval. The melody adapter's
six current timing patterns are a deliberate bounded interface, not an unlimited
composition language. Recurring musical contour in local-model output remains
possible even when orchestration and harmony change.

The saved library is ready for later playlist use and manual selection; automatic
playlist shuffle and grouping separate revisions into one playlist lineage are
not implemented. Existing failed render directories are not blanket-imported.
Fractus's production richness check samples the opening frame; the later-frame
measurements above are extra verification of the observed outputs. Its thresholds
are a minimum-detail floor, not an aesthetic critic, and old art files are not
retroactively deleted.

Wizard rotation and starvation resistance were exercised through the production
scheduler with a continuously replenished creative queue, reaching app then music
study. A complete natural-time pair of Wizard cycles was not observed during the
live verification window; its Night Desk slot is much slower than music updates.

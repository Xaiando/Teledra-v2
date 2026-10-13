# PyMusic restoration and comparison

Implemented in the shared working tree without changing the current live user
score or stopping their running court. The rebuilt release executable is ready
for restart. The running Windows executable was retained as
`target/release/teledra-before-pymusic.exe` so the new build could be installed
without terminating the session.
The previously absent user engine-selection file is initialized to Python with
alternation enabled, so the next restart opens the verified V1 Python baseline.

## User-facing behavior

- `/PyMusic` selects the restored Python editor; `/Synth` selects native Studio.
  Opening either closes the other's owned process tree and stops the old audio.
- `/Music` resumes the selected engine. `/MusicCompare` toggles alternation,
  enabled by default. Each next composition targets the other engine. Completed
  automatic replacements wait for the current pass; manual selection switches
  immediately. One shared playback owner controls both engines.
- Fresh PyMusic sessions use the unchanged V1 `Fivefold Court Engine` source,
  bundled in `Music/v1_music_example.py`. Saved accepted pieces and drafts take
  precedence. Play restores accepted audio even with an incomplete editor draft.
- V1 waveform synthesis, free timed NumPy layering and effects are restored.
  All 72 native voices are also available through `instrument_note`, including
  piano and theremin. This bank is shared with native Court Synth.
- Both editors browse all 72 instruments. Organist dispatch includes the full
  catalog, current source, feedback and `Music/PYTHON_AUTHORING.md`.

## Correctness and review

The direct note bridge never opens a device or runs the grid arranger. Overlays
extend and sum notes without voice stealing. The Python worker captures exactly
one final array, completes execution, resamples correctly and writes full PCM16
stereo at 48 kHz. Rust verifies the WAV, report frame/duration agreement and note
counts before publication. Failed/stale work retains the accepted artifact.

Independent critic findings were fixed: prompt argument/metadata mismatches,
lost shutdown drafts, unjoined reader cleanup, exhausted one-shot replay, ignored
loop flags and hidden worker errors. A whole-waveform proof found and fixed
Windows NumPy int32 overflow during resampling after about one second. Native
downsampling now prevents aliasing, and caught note-helper failures cannot pass
with incomplete counts. Final critic review found no critical blocker.

Renderer tickets, window sessions and cancelled Organist requests prevent stale
work from reopening or controlling another engine. Pause/mute survive completed
handoffs. Source, accepted manifests, drafts and feedback are kept separately.
Only model-origin failures request a bounded correction with exact source and
error, once per accepted Python artifact. Manual editor errors stay local.

## Evidence

- Standard Rust suite: 386 passed, no failures; opt-in environment/model tests
  stay ignored in that suite. Final runtime binary suite: 128 passed.
- 34 Python/UI tests passed, including all 72 short-note voices, the executable
  dispatch-guide example, and exact
  overlap/sample-rate checks. Strict Clippy and release environment check pass.
- Muted operational lifecycle passes: Native → Python → Native, automatic
  completed-pass handoff, stale GUI rejection, failed render retention,
  shutdown draft preservation, accepted replay and completed one-shot replay.
- Existing native transport/persistence/mixer regression passed twice in
  isolation (26.22 s and 25.31 s). An intermittent earlier failure was traced
  to Windows denying atomic status-file replacement in the test-only Tk probe.
  Bounded retry fixes that probe; production transport, assertions and deadlines
  were unchanged. Caller/state/stderr diagnostics were retained.
- Device-free worker proof: corrected `Lantern Rooms`, 80 s, 285/285 helper calls.
  `target/python-music-proof/runtime-1789322096339/accepted/audio.wav`.
- Unchanged V1 source proof: 88 s, 754/754 helper calls, full Rust audio gate.
  `target/python-music-proof/v1-unchanged-1789322151149/audio.wav`.
- Raw details and failed pre-fix audio are retained under
  `target/python-music-proof/verification-notes.md`. Earlier unverified audio
  must not be presented as successful output.

## Remaining musical limitation

Two isolated production model invocations did not establish reliable composition:
one duplicated the duration argument; the guided one returned a bare Python fence,
omitted an import and left long silence. Both failed proof outcomes remain under
`target/python-music-proof/model-study*`. Runtime now recognizes a single complete
Python fence for an explicit Python request and gives executable failures one
bounded correction; these changes do not establish musical quality. No additional
model attempts or manually repaired model audio are claimed as successes.

Helper counts prove requested helper synthesis completed. Arbitrary NumPy code
can still omit a generated array from its final mix; these counts cannot certify
that every intended event was placed. The verified V1 source and technical
playback tests are the dependable comparison baseline, not a listening verdict.

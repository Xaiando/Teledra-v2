# Operational acceptance — 2026-09-06

Continued from the existing working tree and the saved
[September overhaul](ASTRA_OVERHAUL.md). The prior 237-test workspace baseline,
five live model cases, four Queen voice transcriptions, and six-movement musical
proof were retained. Those unchanged checks were not repeated.

## Faults found and repaired

- Pause now applies before the first music render finishes. A later player cannot
  start over that pause request.
- Muting a paused player leaves both the audio and DAW clock paused. Muting or
  unmuting a running player no longer resets only the displayed evolution deadline.
  Space pauses muted playback too. `/MusicMute` provides an explicit command for
  terminals that encode Ctrl+M as Enter.
- A deliberately dismissed DAW no longer produces a broken-pipe error on every
  accepted score. Explicit reopening and recovery after a crash remain distinct.
- The launcher recognizes diagnostic/view arguments anywhere in its argument list,
  forwards them unchanged, and uses the installed build without stopping or
  rebuilding the live court. Normal launches still rebuild and reject build failure.
- Studio ownership uses an OS-held file lock. A duplicate leaves its owner alive;
  a crash releases ownership. Neither studio kills a process named by an old PID
  file, whose PID could have been recycled for an unrelated application.
- The real speech-runtime probe exposed an EOF/process-exit race: a closed audio
  pipe could precede Windows reporting the redirector's exit. Broken writes and
  invalid/truncated PCM now retire and reap the worker before another request.
  Restart preserves the explicit environment and working directory. Empty but
  correctly framed audio produces a visible failure rather than unexplained silence.
- Normal speech shutdown closes stdin and permits a brief graceful exit before an
  exact process-tree fallback. In-flight speech remains invalidated by interruption.
- Terminal reporting now accounts for `NO_COLOR`, independently of ANSI capability.
  The check shell had `NO_COLOR=1`; the old report incorrectly claimed colour was
  active. The opt-out is respected, and the report identifies its cause. With that
  variable removed only for the test process, actual RGB escape sequences were
  observed. Help text now describes Escape and music mute accurately.

## Acceptance evidence

| Check | Result |
| --- | --- |
| Changed Rust crates and integration contracts | 87 tests pass: 61 application, 22 TTS, 4 integration |
| Native audio-device check | Prepared playback stays paused beyond the entire buffer duration; muted playback consumes and finishes the buffer; an unknown device is rejected |
| Real render → effect → player → DAW | Passes first-render pause, mute while paused, persistence, stop/play, and reuse of the same DAW process |
| Hidden real Tk process lifecycle | Hot reload, deliberate dismissal, explicit reopen, crash recovery, and shutdown pass through the Rust supervisor |
| Speech runtime with real audio device and silent PCM fixture | Empty audio reporting, interruption, stale in-flight rejection, crash recovery, corrupt-frame recovery, preserved environment/cwd, and bounded shutdown pass |
| Python checks | Two DAW tests and one subprocess lock-ownership/recovery test pass |
| Real LuxTTS through the revised Rust worker | Both utterances pass: 2.304 seconds / 110,592 samples and 3.285 seconds / 157,696 samples |
| Actual launcher and desktop surfaces | One DAW and one Fractus studio; Fractus acknowledges all 24 animation frames; `/Help` opens and Escape closes it without quitting |
| Diagnostics beside the live court | `Teledra.bat --role organist --check-environment` succeeds; court PID and executable modification time remain unchanged |
| Graceful exit | Ctrl+Q exits successfully and removes the court and four studio Python processes |
| Forced exit | Terminating the isolated test court removes all seven tracked processes, including both console helpers |
| Colour output | Actual foreground and background 24-bit RGB sequences observed with `NO_COLOR` unset for the test process |
| Release | Workspace release build, strict Clippy, formatting, diff whitespace, and final environment check pass |

Live acceptance used an isolated data directory under
`target/astra-verification/launch-smoke-20260906`, Normal mode, and an empty local
audience token. It did not modify the installed court's records, run autonomous
model turns, or perform outreach. Voice quality/model-schema checks from the
previous run were reused. The real worker was rerun because its Rust lifecycle
implementation changed. No test court or worker was left running.

## Saved evidence

All machine logs are in ignored `target/astra-verification/`:

- `continuation-tests.log`, `continuation-clippy.log`,
  `continuation-release-build.log`, and `continuation-environment.log`.
- `operational-runtime.log` contains the three initial passing music/DAW checks
  and the original speech recovery failure. `operational-speech.log` records the
  successful final speech check after repair; the passing music/DAW cases were
  not rerun unnecessarily.
- `operational-daw-python.log`, `studio-lifetime.log`, and
  `continuation-worker-probe.log`.
- `launcher-diagnostics.log`, `launch-snapshot.json`,
  `launch-colour-snapshot.json`, and `terminal-rgb.json`.
- `launch-processes.json`, `launch-cleanup.log`, `forced-exit-processes.json`,
  and `forced-exit-cleanup.log`.

Repeat hardware checks only when a relevant change requires them:

```powershell
cargo test --release -p teledra-music -p teledra-tts -p teledra --lib --bin teledra operational_ -- --ignored --nocapture --test-threads=1
.\.venv\Scripts\python.exe -m unittest discover -s Music -p test_daw.py
.\.venv\Scripts\python.exe -m unittest discover -s Sidecars/scripts -p test_studio_lifetime.py
cargo run --release -p teledra-tts --example worker_probe
cargo build --release --workspace
.\Teledra.bat --check-environment
```

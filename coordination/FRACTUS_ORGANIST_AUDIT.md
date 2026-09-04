# Fractus and Organist execution-path audit

Audit date: 2026-08-02. The behavioral reference is the installed V1 tree at
`D:\Teledra`, with the linked repository commit recorded in `PARITY_GAPS.md`.
The audit follows ownership and process boundaries, not merely command names.

## Fractus

| Boundary | V1 behavior | V2 before this audit | V2 replacement |
| --- | --- | --- | --- |
| Generation | Python Fractus DSL/model/render stack | Bounded Rust DSL and verified PNG/GIF renderer | Keeps the safer native renderer and its 36 families/layers |
| Presentation | Persistent Tk Fractus application | `explorer.exe` opened the artifact in a generic Windows viewer | One supervised `Fractus/studio.py` living gallery |
| Animation | GUI frame loop and animated GIF export | GIF phases existed on disk but no Fractus-owned player was launched | GIF phases play in Studio; PNGs receive subtle presentation-only motion |
| Reload | Command mailbox hot-reloaded the open GUI | Every artifact was opened independently | Durable JSON mailbox hot-reloads the scene in the same window and returns a matching acknowledgement |
| Controls | Play/Pause plus editing/export surface | No app controls | Play/Pause and Fit; verified source and manifest are already persisted automatically |
| Safety/lifecycle | Status mailbox and a tracked GUI child | Generic shell process was not a Fractus lifecycle | Trusted-root/extension/size/decoded-frame bounds; command/status IDs, child restart, kill, and reap |

The V2 improvement is separation of concerns: the gallery can animate and
present, but cannot execute edited Python or modify the verified artifact. DSL
changes still enter through Teledra and are revalidated by Rust.

## Organist

| Boundary | V1 behavior | V2 before this audit | V2 replacement |
| --- | --- | --- | --- |
| Startup | Existing CourtScore opened with `--play` | TUI displayed music ON but did not call playback | Last valid score restores, verifies, plays, and opens DAW at normal boot |
| Synthesis | Large Python Court Synth application | Bounded native score renderer and `rodio` player | Keeps native verification as the sole audio engine |
| DAW | Persistent GUI with transport/score visualization | No Organist application | One supervised `Music/daw.py` visual DAW |
| Transport | Play controls in Court Synth | TUI commands only | DAW Play/Pause/Stop route back to the Rust player |
| Evolution | Composer updated the current CourtScore | Ctrl+U/model score actions existed | Native verified change every piece; DAW Evolve and Ctrl+U trigger it immediately; model composer remains additive |
| Persistence | Current score/project files | Score lived only in process memory | `current_score.json` and verified `current.wav` restore across sessions |
| Audio ownership | Court Synth GUI owned audio | Rust player owned audio when manually started | Rust remains the one audio source; DAW never duplicates playback |

The 2026-08-28 major DAW pass added a typed verified mixer, reversible mute,
gain/pan edits, keep/reject feedback, selectable 24/36/48-second development
cadence, New Movement composition, chord/section timeline, craft analysis, and
a six-axis native development cycle. Sparse installed scores are mastered into
performance arrangements before playback; the original authored parts are
preserved and only missing functional seats are added or duplicate roles
replaced.

## Verification evidence

- Python compilation passes for both Studio applications.
- The production renderer produced four consecutive 600x800 GIFs with 24/24
  distinct frames each; every manifest reported `verified: true`.
- The first live audit caught the venv redirector swallowing Studio stdin. The
  replacement durable mailbox then returned a matching `loaded` acknowledgement
  for the exact artifact path and all 24 decoded frames.
- Two captures of the actual Studio canvas 850 ms apart changed 204,941 pixels
  (33.9% of the canvas), proving that the open app was drawing and animating.
- Visual captures verified the gallery canvas and DAW transport, meters,
  sequencer cells, revision, and live playhead render on this machine.
- Three isolated native music variations had unique score/PCM hashes,
  0.189--0.197 waveform-difference RMS, and near-zero inter-version correlation.
- The accelerated live runtime advanced through successive unique PCM digests
  while exactly one DAW and one Fractus Studio remained responsive.
- A final desktop-shortcut launch with no timing overrides advanced the restored
  92 BPM score to an 87 BPM verified variation after the ordinary 45-second
  bound; score and PCM digests both changed.
- The workspace test suite, formatting, Clippy with warnings denied, and the
  optimized workspace release build pass (134 tests at this audit).

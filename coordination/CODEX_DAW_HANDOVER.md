# Codex handover: Organist DAW

**Prepared:** 2026-08-03; major overhaul verified 2026-08-28  
**Workspace:** `D:\Teledra V2`  
**Current state:** major v3 workstation overhaul implemented and machine-verified

## Pickup objective

Continue improving the Organist DAW without breaking the single-source audio
contract. Keep Rust responsible for score validation, synthesis, verification,
playback, and persistence. Keep Python responsible for the visual workstation
and operator controls only.

Start by reading this file, then `Music/README.md` and
`coordination/FRACTUS_ORGANIST_AUDIT.md`. Do not copy the monolithic V1
`court_synthesizer.py` into V2; use `D:\Teledra` only as a read-only behavioral
reference.

## Delivered behavior

- Normal Teledra startup restores the last valid Court Score, or falls back to
  the bounded default score.
- The score renders off the TUI thread, passes native acoustic verification,
  starts one looping `rodio` player, and opens one Organist Studio window.
- The DAW hot-reloads later Organist scores without opening another window.
- Play, Pause, Stop, Evolve Now, New Movement, Keep/Reject, cadence, gain, pan,
  and reversible mute actions travel over a typed newline-delimited protocol.
- The DAW shows movement identity/countdown, arrangement sections, chord
  timeline, craft score/checklist, scrollable mixer, sequencer, animated
  meters, transport state, and accurate pause/resume playheads.
- The verified score and audio persist under the V2 data root as
  `music/current_score.json` and `music/current.wav`.
- Rust creates a verified variation every 18--36 seconds by default. Studio can
  select 24, 36, or 48 seconds. This guarantee does not depend on the model
  returning a valid or changed score.
- Evolution is a six-movement cycle: pulse, motif, counterpoint, texture,
  harmony, and modulation. Only the named axis is rewritten; tonic/tempo remain
  anchored until modulation and progressions retain a diatonic loop cadence.
- The arranger develops every melodic phrase as statement, answer,
  development, and reprise, and its phrase-ending drum fills now create real
  transition hits instead of being unreachable code.
- Every score receives a persistent 0--100 craft report across pulse,
  foundation, harmony, melody, cadence, space, palette, and form.
- Ctrl+U and DAW Evolve create that native variation immediately. The separate
  model-composer nudge remains on its slower cadence for larger creative jumps.
- Accepted revisions retain the last 64 score snapshots under `music/history/`.
  `music/evolution.jsonl` records independent score and PCM digests, duration,
  tempo, key/mode, title, and revision for every playback replacement.
- `TELEDRA_DISABLE_MUSIC` intentionally suppresses automatic startup music.
- Closing Teledra requests a graceful DAW shutdown, then uses an exact bounded
  process-tree fallback if the surface does not exit.

The 2026-08-03 machine proof evolved the installed score three times. All
outputs passed acoustic verification and had different score/audio hashes:

| Revision | BPM / key | Score digest | PCM digest | Difference RMS | Correlation to previous |
| --- | --- | --- | --- | --- | --- |
| 1 | 87 / A# minor | `b39482ffb0d132ca` | `ba71b6f27f5a4a20` | 0.1924 | -0.0027 |
| 2 | 96 / C# major | `1e0b13452339774e` | `d75402e151d58ec7` | 0.1891 | -0.0027 |
| 3 | 89 / G# dorian | `d1f48371b5fee6d1` | `aa8949049950f3c2` | 0.1974 | 0.0005 |

The near-zero correlations rule out a renamed/copy-only loop. The live runtime
then accepted successive revisions 25--29 with a different PCM digest at every
transition while one DAW window remained responsive.

The 2026-08-28 overhaul replayed the installed 4-track, 32-bar lineage through
the new mastering boundary. Its objective craft report rose from 52/100 to
100/100 by preserving the cello/reed/flute material while restoring a bounded
pulse, chordal middle, tonal cadence, instrumental palette, and 16-bar live
form. Six consecutive named movements all remained 100/100, passed acoustic
verification, produced distinct score and PCM digests, measured difference RMS
0.179--0.195, and held correlation to the previous render between -0.062 and
0.073. No copied or cosmetic-only variation survived.

Final installed acceptance used the desktop shortcut with no cadence override:
the restored 92 BPM revision 0 (`91c3411fd1accb04`) advanced after the normal
45-second bound to 87 BPM revision 1 (`95ffae260c06c026`). The TUI, DAW, and
Fractus Studio remained responsive.

## Ownership map

| File | Responsibility |
| --- | --- |
| `Music/src/lib.rs` | Court Score schema, bounds, deterministic evolution, verification, WAV writer, `Player` |
| `Music/src/arrange.rs` | Section, harmony, groove, and playable-note arrangement |
| `Music/src/synth.rs` / `effects.rs` | Instrument synthesis and master/effect chain |
| `Music/daw.py` | Tk DAW layout, visualization, playhead/meters, JSONL control output |
| `Teledra/src/runtime/daw.rs` | One supervised DAW process, stdin/stdout protocol, cleanup |
| `Teledra/src/runtime/mod.rs` | Startup score restore, persistence, playback replacement, DAW effects |
| `Teledra/src/runtime/actions.rs` | Background score render and `MusicReady` effect |
| `Teledra/src/runtime/commands.rs` | `/Music`, `/MusicOff`, toggle, evolve, keep/reject commands |
| `Teledra/src/main.rs` | Automatic Organist startup and tool-effect drain |
| `Agents/profiles/organist.md` | Organist composition policy and bounded `[COURT_SCORE]` contract |

Useful symbols are `Score::evolved`, `verify_audio`, `start_music`,
`evolve_music_now`, `evolve_music_if_due`, `persist_current_score`, and
`FractusStudio::show`. Search symbols rather than relying on line numbers.

## Runtime flow

```text
startup or /Music
  -> background Score::render + verify_audio
  -> ToolEffect::MusicReady
  -> persist current_score.json/current.wav
  -> replace the single Rust Player
  -> hot-reload the single Organist Studio

DAW Play/Pause/Stop/Evolve
  -> JSONL stdout
  -> ToolEffect::DawAction
  -> Rust player transport or immediate native verified variation

accepted playback replacement
  -> current_score.json + current.wav
  -> history/variation-<revision>-<digest>.json (last 64)
  -> evolution.jsonl with independent score and PCM digests
```

## JSONL protocol

Teledra to DAW:

```json
{"action":"score","score":{"title":"..."},"transport":"playing","cadence_seconds":36,"analysis":{"quality":100,"grade":"Masterwork"}}
{"action":"transport","state":"paused"}
{"action":"cadence","seconds":24}
{"action":"shutdown"}
```

DAW to Teledra:

```json
{"action":"play"}
{"action":"pause"}
{"action":"stop"}
{"action":"evolve"}
{"action":"new_movement"}
{"action":"keep"}
{"action":"reject"}
{"action":"set_cadence","seconds":24}
{"action":"set_track_gain","track":2,"gain":0.42}
{"action":"set_track_pan","track":2,"pan":-0.25}
```

Only the typed variants above are accepted by the Rust supervisor. Mixer
numbers are independently bounded again in Rust before a cloned score is
rendered.

## Non-negotiable constraints

1. Never add audio playback to `Music/daw.py`; it would duplicate the Rust
   stream and make OBS routing unreliable.
2. Never execute model-authored Python, shell, MIDI scripts, or arbitrary file
   paths. Organist output must remain a bounded typed Court Score.
3. Keep render and device work off the TUI thread.
4. A new score must pass `Score::validate` and `verify_audio` before playback.
5. Preserve ticket/epoch handling so stale background renders cannot replace a
   newer score.
6. Preserve one DAW window with hot reload and exact child cleanup.
7. Do not kill all Python processes during development; match the Teledra V2
   executable/script path and verified process tree.

## Honest current boundary

Organist Studio is now an active verified mixer and composition surface.
Per-track gain, pan, and reversible mute are implemented. Direct note-grid
editing, automation curves, solo buses, patch editing, MIDI input, and manual
timeline rearrangement are not implemented. Composition changes come from the
named native movement cycle or the Organist's validated complete-score
response. Any future editor must retain the typed Rust-first, render, verify,
and atomic-replacement contract.

Recommended next increment if requested:

1. Add typed note-cell edits with an explicit undo stack.
2. Add automation lanes as bounded score data rather than Python-owned state.
3. Add a solo audition bus without persisting destructive zero-gain edits.
4. Add MIDI import through a fixed parser with note/count/duration bounds.

## Verification and release

From `D:\Teledra V2`:

```powershell
.\.venv\Scripts\python.exe -m py_compile Music\daw.py
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release --workspace
```

At handover, all 210 workspace tests, the Python self-test/compilation, formatting, strict
Clippy, and the optimized release build pass.

Live acceptance:

1. Launch `C:\Users\Kaged\Desktop\Teledra V2.lnk`.
2. Confirm exactly one window titled `Organist // Court Synth DAW` is
   responsive and music begins automatically.
3. Exercise Play, Pause, Stop, and Play again; after Stop, Play may rerender the
   persisted current score before resuming.
4. Move gain and pan, toggle mute twice, and select each cadence preset. Confirm
   the mixer is audible, mute restores the previous gain, and the countdown
   follows the selected cadence.
5. Exercise Evolve Now and New Movement. Confirm movement/craft displays,
   revision, tempo/key, score digest, PCM digest, `current_score.json`, and
   `current.wav` update while the same window remains.
6. Exercise Keep and Reject and confirm feedback is accepted without disrupting
   playback.
7. Inspect `music/evolution.jsonl`; consecutive accepted variations must not
   share either digest.
8. Exit Teledra with Ctrl+Q and verify both the venv launcher and its base
   Python DAW child are gone.

The installed shortcut points to `D:\Teledra V2\Teledra.bat`, which selects
the repository `.venv` and `target\release\teledra.exe`.

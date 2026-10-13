# Handover

Latest conversation work: [2026-09-18 YouTube and podcast sessions](YOUTUBE_PODCAST_20260918.md).

Speech and conversation update: [2026-09-17 noise and continuity](SPEECH_CONTINUITY_20260917.md).

Latest: [2026-09-06 operational acceptance](OPERATIONAL_ACCEPTANCE.md).

For the current implementation and verification, start with
[the 2026-09-05 overhaul](ASTRA_OVERHAUL.md). The observations below are historical;
composition now uses schema-constrained output and correction is bounded per turn.

Written after a session spent running the court rather than reading it. Nearly
everything below was found by watching the live transcript, the rendered audio,
and the process table — not by inspecting source. The tests all passed the whole
time.

## The one thing worth internalising

`qwen2.5:7b` does not reliably obey prose instructions about output format, and
it does not stop disobeying when told more firmly. Over one session it wrote a
score as `[COURT_SCORE: {json}]`, then `**COURT_SCORE: {json}`, then markdown
headings, then a fenced JSON block with no tag at all, then a JSON array. Each
fix held; the model moved.

Build for non-compliance rather than against it:

1. **Recover** the shapes already seen (cheap, one parse).
2. **Correct** what will not parse — hand the error back and ask again, bounded
   to one attempt per payload.
3. **Salvage** when the intent is unambiguous. A composer nudge that returns a
   JSON object without a tag is a score; take it.
4. **Reject no-ops.** Asked to evolve a score while being shown that score, the
   Organist returned it verbatim three times running. An unchanged result is a
   failure, not a result.

The same shape applies to speech. Identical prompts produce identical replies,
which then enter the history and reinforce. Vary the request, name the last
answer back, suppress the duplicate, and loosen sampling for idle turns.

## Measure it, do not assume it

Two harnesses exist because "it looks fine" was wrong every time:

```powershell
cargo run -p teledra-court --example replay_transcript -- "$env:LOCALAPPDATA\Teledra V2\memory\court_transcript.jsonl"
cargo run --release -p teledra-music --example arrangement_report
cargo run --release -p teledra-music --example render_demo -- court.wav
cargo run -q -p teledra-fractus --example payload_check
cargo run --release -p teledra-music --example evolution_walk
cargo run --release -p teledra-music --example instrument_audit
cargo run --release -p teledra-music --example voice_load
```

The first answers "what fraction of the court's requests actually execute" — it
was 15 of 387 turns before the recovery work. It reports parsed actions *and*
bare JSON artifacts separately, because a score now commonly lands by salvage
rather than by tag: a run showing `0 parsed actions, 49 salvaged` is a working
court, not a silent one. The second answers "what fraction of written notes are
sounded" — it was 62%. Run them against real transcripts, not fixtures.

`verify_audio` now reports `periodicity` and `dynamic_range` alongside peak and
RMS. A render can be loud, unclipped, and still be a wash; those two numbers are
what catch it.

## Things that cost hours, so they do not cost yours

- **`Teledra.bat` used to build only when the exe was missing.** A whole stream
  ran on a stale binary with the fixes present in the tree. It now always
  builds; cargo is a no-op in under a second.
- **`.venv\Scripts\python.exe` is a redirector stub** that re-executes the real
  interpreter as a child. Every sidecar shows as two processes — that is normal,
  not a leak. It also means the process the runtime holds is the stub, so
  killing it can orphan the real window. The studios record their own window and
  close the previous one; do not undo that.
- **Destructors do not run on abnormal exit.** Closing the console window, a
  crash, or an external kill all skip `Drop`, which is how six Python children
  came to be orphaned holding the microphone. A job object now enforces the
  reap in the kernel (`Teledra/src/lifetime.rs`). Verified by hard-killing the
  parent: five children, zero survivors.
- **Colour depends on how the court is started.** The terminal backend probes
  for ANSI support once and caches it; launched from a script or scheduler that
  probe can fail and every `Color::Rgb` collapses to the default, rendering the
  whole court white on black and the portrait as a field of blocks. Start it
  from the shortcut. `terminal.rs` sets the console mode first, which helps but
  did not fully cover `Start-Process`.
- **Silent fallbacks hide everything.** Fractus accepted any string as a palette
  and fell through to the default, so a paragraph of the Artist's prose became
  the palette and every artwork rendered identically while reporting success.
  Unrecognised things must be refused or reported, never quietly substituted.

## The court was dying for a reason the tests could not see

Three hundred consecutive live turns ran **76% Queen**, in runs of up to sixteen,
and most of those runs were withheld by the repetition guard. Every layer looked
healthy: the guard worked, sampling was loosened, the scheduler was fair. The
cause was one line in `begin_autonomous_turn` — a locked topic sent **the
identical prompt to the same voice** for up to twenty turns:

```
[20:08:15] queen << Continue the court's locked thread about 'Echoes of Serenity'...
[20:08:35] queen << Continue the court's locked thread about 'Echoes of Serenity'...
[20:08:59] queen << Continue the court's locked thread about 'Echoes of Serenity'...
```

Identical prompt, near-identical answer, withheld as a repetition, next turn.
The guard was doing its job perfectly and the audience heard silence.

Read the `user` field of `court_transcript.jsonl`, not just `assistant`. It
records the exact prompt every turn received, and it is the fastest way to tell
"the model is stuck" from "we keep asking the same thing".

Three fixes, all in `Teledra/src/app.rs` and `main.rs`:

- **`COURT_ROTA`** — the open floor goes round the court. The Queen presides at
  half the turns and never speaks twice running; each minister gets a request
  written for their office (`minister_angle`), not the Queen's prompt in a
  different costume.
- **A locked topic is a subject, not a speaker.** The rota still applies, the
  angle still varies, and `avoid_clause()` now reaches that branch too.
- **A launched court performs.** Startup mode was `Normal`, which answers the
  operator but never speaks first; only a saved Restream token or a typed
  command moved it off that. With no token on disk a fresh launch sat silent —
  music evolving, gallery rendering, court mute. It now starts in `Babble`,
  overridable with `TELEDRA_START_MODE`.

Measured after: 12 turns, Queen 4, no consecutive repeats, the Queen addressing
the Organist by name and the Organist answering.

## Instruments the court can actually reach

Six acoustic voices were added — organ (drawbar-additive), piano, flute, reed,
cello, harp — each with its own `natural_range()`, and pitches are octave-folded
into it rather than refused. The default score is now a seven-part ensemble
measuring flatness 0.008 / harshness 0.016, at or better than V1's median.

That work was nearly worthless, and the reason is the useful lesson. The live
court was **4,275 variations deep** on a lineage voiced by the six synths its
first score happened to name. Native evolution rewrote notes, rhythm, harmony,
and gain — but never *who plays*. A new instrument reachable only from a fresh
model composition is not an instrument the court can use, because a fresh
composition essentially never happens.

`Instrument::family()` groups voices by role and `evolved()` re-voices within
that role every `ORCHESTRATION_VARIATIONS` (5). Percussion has an empty family:
a kick that became a snare would be a different groove, not a different colour.
Track names are rewritten to match, because `from_name` reads them back.

```powershell
cargo run --release -p teledra-music --example evolution_walk -- 40 "$env:LOCALAPPDATA\Teledra V2\music\current_score.json"
cargo run --release -p teledra-music --example render_demo -- live.wav "$env:LOCALAPPDATA\Teledra V2\music\current_score.json" 17
```

Both now take the **live score**, not just `default_score()`. Walking the real
4,275-deep lineage forward is what proved the roster was frozen; walking it
after the change showed cello, organ, harp, piano, reed and bell arriving within
half an hour of stream, with key/tempo still anchored and the groove untouched.
Verified in the running court at variation 4285: `bass keys pluck` became
`sub_bass organ piano`.

## Windows Smart App Control blocks freshly-built test binaries

Twice in one session `cargo test --workspace` failed with:

```
could not execute process `...\deps\teledra-<hash>.exe` (never executed)
Caused by: An Application Control policy has blocked this file. (os error 4551)
```

`VerifiedAndReputablePolicyState = 1` under
`HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy` — Smart App Control is on and
enforcing. It blocks unsigned executables with no reputation, which is exactly
what a just-compiled test binary is. It is **not** a Rust version problem: this
workspace is pinned to 1.95.0 by `rust-toolchain.toml` and both blocks happened
on 1.95.0.

It is intermittent and hash-based. The workaround that worked both times:

```powershell
cargo clean -p <package> --release
```

Then re-run. Do not read the misleading cargo summary — a blocked binary is
reported under whichever test target cargo names last, so the failure gets
attributed to an innocent suite that passes in isolation. Count the totals: the
workspace now runs **210 tests**, and a run reporting far fewer has aborted
early.

## Still open

- **The correction loop has not been observed firing in production.** It is
  unit-tested and its trigger is pinned by an integration test using the real
  markdown-score output, but no malformed payload has arrived since it shipped.
- **The Queen keeps a standing salutation.** "Ahem! Attention all subjects"
  opens most lines despite an explicit rule in `queen.md` and the prompt quoting
  the opener back. Content varies and nothing is suppressed, so it is cosmetic —
  but it is the clearest evidence that prose rules alone will not hold. The rota
  reduces how often it is heard; it has not removed it.
- **The Artist can burst.** Five turns in five seconds were observed after the
  rota landed, four of them "acted without speaking" — action tags each drawing
  a follow-up. It predates the rota and is not obviously wrong, but it is the
  next thing worth measuring with `replay_transcript`.
- **Resolved 2026-08-28: Night Desk no longer gives the Queen four of every six
  backstage turns.** It has a separate 12-step, role-specific rota covering all
  11 offices; only the original Treasury update remains audible. Its counter is
  independent, so maintenance cannot skip the next public speaker.
- **Resolved 2026-08-28: `COURT_ROTA` now includes Alchemist, Malthus, and
  Wizard.** Each has a request written for its office, and an executable test
  requires every court member to receive the floor.
- **Voice timbre still remains the operator's judgment, but the segment leak is
  mechanically closed.** Local word-timestamp transcription proves every Queen
  prompt extends beyond its final word. In particular, clean-mode
  "magnificently" ends at 5.52 seconds while the encoded prompt continues to
  6.72 seconds; custom mode contains no such word and retains 0.41 seconds after
  its complete final phrase. The hard-clipping defect in `Voice::level` is also
  fixed.
- **Music now has two evolution layers.** Native deterministic evolution runs
  at the end of each piece, bounded to 20–45 seconds, and never waits for model
  compliance. `TELEDRA_MUSIC_VARIATION_SECS` can override the cadence. The
  separate model-composer nudge still defaults to five minutes through
  `TELEDRA_MUSIC_EVOLVE_SECS`; it is additive rather than a liveness dependency.
- **Resolved 2026-08-28: the same-name standalone products are no longer thin
  owners.** Kraken now has its durable queue, daemon/Hub workers, typed skills,
  transactional repair forge, citation gates, and real browser graduation;
  its read-only V1 inventory run reproduced 15/16 browser acceptances.
  FrankenSwarm now has immutable model/download locks, semantic autopsy,
  tournament/evaluation, SQLite runtime, and a hardened sandbox. Its current
  donor set is truthfully blocked by token ID 100256 rather than waved through
  with the V1 mock tensor report. Heavy tools and model weights remain a live
  provisioning gate; see `PARITY_GAPS.md`.

## Read next

`AUDIT.md` — a measured audit of the whole system as of 27 August, ranked by
what the audience notices. The top findings (the Artist's direction never
reaches the canvas, Fractus has no retention, the autonomous court never
recalls memory, a month of work is uncommitted) are each one count away from
obvious and none of them is in this file.

## Conventions worth keeping

Comments explain *why*, and several carry the specific evidence that motivated
the code — the count of discarded scores, the number of identical proposals, the
line that played nine times. Keep that when editing; it is the difference
between a rule someone trusts and one they delete.

Bounds everywhere: one retry, one correction per payload, one window per studio,
one court per machine.

# V2 audit — 27 August 2026

Measured, not read. Every number below came from the live data root
(`%LOCALAPPDATA%\Teledra V2`), the transcript (771 turns), the evolution ledger
(4,437 entries), the Fractus gallery (1,178 GIFs), git, and the crate tree —
not from what the code says it does. Where a claim rests on inference rather
than a count, it says so.

Ranked by what the audience notices first, then by what will hurt next.

## Snapshot

| | |
|---|---|
| Rust | 29,600 lines across 15 crates, 208 tests, clippy `-D warnings` clean |
| Toolchain | pinned 1.95.0 via `rust-toolchain.toml`; Smart App Control now **off** (was blocking build scripts; irreversible) |
| Git | last commit **27 July**; **88 dirty/untracked paths** — a month of work, uncommitted |
| Court (post-rota, 56 turns) | Queen 46 %, longest run 2, 0 empty turns, 3 near-duplicates |
| Music | variation every 48 s median; 174/199 gaps in the 30–90 s band |
| Public feed | machine payloads 32 → 0 over 740 turns |
| Data root | 3.5 GB, of which Fractus 3.3 GB |

---

## 1. The Artist never reaches the canvas  *(critical, audience-visible)*

The Artist talks about art on every turn and none of it is rendered.

- Last 40 Artist turns: **19 ask for a mandala, 14 for lissajous** — both are
  valid Fractus kinds. Last 300 manifests: **zero** of either.
- Artist turns carrying a `[FRACTUS_ART:` tag: **0 / 40**.
- What is rendered is a hard-coded rotation — `FAMILIES`, `ACCENTS`, `PALETTES`
  (six each) in `Teledra/src/runtime/mod.rs:681–697` — driven by
  `start_fractus_gallery`. The manifest distribution is flat across exactly
  those six kinds (64/49/45/44/43/42) and six palettes. That is a metronome,
  not an artist.
- Consequence: the last 200 GIFs have **19 distinct** first-64 KB hashes; one
  image repeats 45 times.

The Artist is the only minister whose prose is honest about intent and whose
work is 100 % discarded. It is the mirror of the Organist problem that was
fixed today (score written in prose, not in the schema): the model narrates
instead of tagging. Apply the same shape of fix — recover the intent from
prose. "the Mandala family … neon sunset palette" is unambiguous; salvage it
into a `Fractus` action the way an untagged JSON score is salvaged into a
`COURT_SCORE`. Then let the Artist's action pre-empt the gallery's next
rotation slot rather than compete with it.

**Acceptance:** over 40 Artist turns, ≥ 50 % produce a Fractus action, and
the manifest kind matches the family the Artist named.

## 2. Fractus has no retention  *(critical, operational)*

1,178 GIFs, **3.49 GB**, oldest 32 days, median 3.2 MB each, ~1 GB/week. Music
history is pruned to 64 (`prune_music_history`); Fractus has no equivalent —
the only `remove_file` in `runtime/fractus.rs` clears the command/status
files. Left alone this fills the disk.

**Fix:** prune to the newest N (64 matches music) on every accepted render.
Dedupe by content hash while at it — with #1 unfixed, 90 % of them are the
same six images.

## 3. Night Desk is mostly talk  *(high)*

110 Night Desk turns; **15 carried an action tag** (14 %). The other 86 % are
prose about work that did not happen — "I will access the digital archives…"
with nothing behind it. The three fixed prompts ask for "one concrete action
tag when useful"; the model reads "when useful" as "never".

Also: the Night Desk speaker branch still gives the Queen 4 of every 6
backstage turns with three fixed prompts — the same shape as the locked-topic
bug fixed today, and the same model.

**Fix:** treat a Night Desk turn with no action as a no-op and say so
backstage (the correction loop already knows how to ask again, bounded to
one retry). Route maintenance to Archivist/Scribe, study to Diplomat, creation
to Artist/Organist, and give each its own request.

**Acceptance:** ≥ 60 % of Night Desk turns produce an action.

## 4. The proposals queue is a graveyard  *(high)*

**75 proposals, all `pending`, median age 32 days**, oldest 33. 64 are Moltbook
comments the Diplomat drafted for posts a month old. Nothing expires them,
nothing surfaces them, and the header's `SUGGESTIONS: 73` has been a constant
since the first week. A counter that never changes is not a signal.

**Fix:** expire after 7 days (Moltbook comments after 48 h — the thread has
moved on). Show the count of *fresh* proposals only. Consider whether the
Diplomat should draft comments at all when no operator has ever approved one.

## 5. The autonomous court has no memory  *(high)*

`recall_into` is called from exactly one place: the operator pressing Enter
on non-command text (`main.rs:371`). Every autonomous turn — which is every
turn on a stream — runs with an empty `memory_context`. The Memory crate is
1,053 lines with an audience ledger, taste, desire, and decay, and the court
never consults it unless a human types.

Research is the same: `ResearchService` exists, `[RESEARCH:` appears in
**0 of 771** replies, and `research/briefs.jsonl` was last written 8 days ago.

**Fix:** recall on every turn, keyed on the locked topic or the last public
line. Budget it (the prompt is already long) but do not skip it.

## 6. The schema is leaking into the tag vocabulary  *(medium)*

Tag heads seen across all turns: `FACT` 11, `ARCHIVIST` 9, `DELEGATE` 5,
**`BPM` 5, `TITLE` 3, `BARS` 3, `TONIC` 3, `MODE` 3, `SWING` 2, `SIDECHAIN` 2,
`PROGRESSION` 1**. The Organist has learned that square brackets mean "field"
and writes `[BPM: 96]` as if it were an action. These land in `unrecognised`
and are reported backstage, which is correct — but they also mean the model
never sees a whole score succeed by tag, so it keeps inventing shapes.

`[FACT: S1]` is different: it reaches **public speech** (6 of the last 300
replies open with it) because it is not a recognised head and not a payload.
It is a citation format the model picked up from the research prompt.

**Fix:** strip `[FACT: …]` markers from visible text (keep them backstage as
provenance). For the field-tags, the one-shot in `organist.md` should show a
*complete* tagged score, not a schema description; the model copies shapes,
not rules.

## 7. A month of work is uncommitted  *(critical, engineering)*

`git log -1` → 27 July. `git status --porcelain` → 42 modified, 46 untracked.
Everything Codex has built and everything from today's session — rota,
re-voicing, six instruments, payload stripper, build identity, launcher
guard, Smart App Control diagnosis — exists in one working tree on one disk.

**Fix:** commit. Then commit at the end of every session. This is not a
style point; it is the only backup.

## 8. Large crates behind single commands  *(medium)*

| crate | lines | references from `Teledra/src` | reached by |
|---|---|---|---|
| Kraken | 4,823 | 2 | `/kraken` only |
| Frankenswarm | 2,940 | 3 | `/panel` compose only |
| Tower | 310 | 1 | constructed, never read |
| Workshop | 452 | 2 | `workshop/` dir is 0 bytes |
| Swarm | 292 | 2 | `missions/` dir is 0 bytes |
| Memory | 1,053 | 1 | see #5 |
| Research | 573 | 1 | see #5 |

`HANDOVER.md` and `PARITY_GAPS.md` describe both crates accurately as
standalone products. The gap is not the code; it is that 7,700 lines of
Kraken and Frankenswarm are reachable from the court only through two slash
commands nobody has typed on stream. Either wire them into an autonomous cadence (Night Desk is the
obvious home — see #3) or stop counting them as parity.

## 9. Operator surface  *(medium)*

**101 slash commands** in `app.rs`. `/help` is not one of them. Nine begin
with `/test`. There is no way to discover them from inside the court, and
several are aliases (`/night`/`/nightdesk`, `/mic`/`/microphone`,
`/yt`/`/youtube`, `/pymusic`/`/pythonmusic`).

**Fix:** a `/help` that lists them grouped, and a pass to retire aliases.

## 10. Test density is uneven  *(low)*

Sidecars 280 lines / 1 test. Tower 310 / 2. Swarm 292 / 2. Workshop 452 / 2.
Memory 1,053 / 5. Against Court 1,382 / 28 and Music 4,875 / 35. Every bug
found today was in a well-tested crate and was found by *measuring live
behaviour*, not by a unit test — which is the argument for more measurement
harnesses (see below), not more unit tests.

~175 `unwrap()` calls outside test modules (approximate). Three real `unsafe`
blocks, all Win32 FFI (`lifetime.rs`, `terminal.rs`), all justified.

---

## What is working — leave it alone

Post-fix live evidence, 56 turns since 09:50 today:

- **Rota:** Queen 26/56, seven other voices heard, longest consecutive run 2
  (was 16), 0 "acted without speaking", 3 near-duplicate openings (was ~1 in 4).
  The Queen addresses ministers by name in 13 of 26 turns and they answer.
- **Music:** re-voicing verified live (`bass keys pluck` → `sub_bass organ
  piano` at v4285); key/mode anchored; variation cadence steady.
- **Feed hygiene:** 32 leaking replies → 0 over the full transcript, with
  labelled prose preserved (three regression tests pin the exact shapes).
- **Startup:** Babble on launch, terminal capability reported (`Display` row),
  build identity in the backstage trace, launcher kills the previous court
  before building so the stale-binary path is no longer reachable by accident.

## Unverified — needs an ear or an eye, not a script

- **TTS quality.** `--check` confirms the worker, model caches, and nine
  reference voices are present. Whether the custom voice sounds right is the
  operator's judgment; nothing on disk answers it.
- **The monochrome portrait.** Two screenshots showed it; both may have been
  stale binaries (the second definitely was — `Press any key to continue` is
  the launcher's failure path). The renderer is verified correct (`--portrait`
  dumps a clean colour image) and the session reports 24-bit. If the
  PROTOCOLS panel shows `Display  NO COLOUR` in red, it is real; if it shows
  no `Display` row at all, the binary is stale.
- **Queen's salutation.** "Ahem" opens 14 of 56 post-fix replies. Down from
  most, not gone. Cosmetic; prose rules will not remove it.

## How to re-measure

```powershell
# Feed hygiene across the whole transcript (leaks before/after, innocent cuts)
cargo run --release -p teledra-court --example sanitize_check -- "$env:LOCALAPPDATA\Teledra V2\memory\court_transcript.jsonl"
# What fraction of requests execute; salvaged vs parsed
cargo run -p teledra-court --example replay_transcript -- "$env:LOCALAPPDATA\Teledra V2\memory\court_transcript.jsonl"
# Walk the live lineage; key/tempo/voicing cadence
cargo run --release -p teledra-music --example evolution_walk -- 40 "$env:LOCALAPPDATA\Teledra V2\music\current_score.json"
# What the portrait panel actually draws
.\target\release\teledra.exe --portrait check.png 85 44
```

Worth adding, in this order: a `gallery_check` example (Artist intent vs
manifest kind, distinct-hash ratio), a `nightdesk_yield` count, and a
proposals age histogram. Each of the top findings above was one such count
away from being obvious.

## Suggested order

1. Commit (#7). Ten minutes; removes the only catastrophic risk.
2. Fractus retention (#2). Small, stops a disk filling.
3. Artist salvage (#1). The largest audience-visible gain; the pattern
   already exists for scores.
4. Recall on autonomous turns (#5). Unlocks a crate that already works.
5. Night Desk yield + rota (#3). Same fix as today's, one branch over.
6. Proposal expiry (#4), `[FACT:]` stripping (#6), `/help` (#9).

---

## Organist / DAW / synth review — after the checkpoint merge

Checked against the merged tree (`77796a1`), the live lineage
(`Harmonic Resonance`, v4348), the ledger, and every harness. The merge is
coherent: the six-movement cycle (pulse → motif → counterpoint → texture →
harmony → modulation) absorbed the re-voicing axis (fires on *Texture*) and
the key/tempo anchor (moves on *Modulation*, every sixth variation). One
thing changes at a time, which is the right shape.

**Verified working**

- 215 workspace tests, clippy `-D warnings` clean; `daw.py --self-test`
  passes; DAW protocol is typed and refuses free-form actions.
- `evolution_proof`: three consecutive movements, distinct score and PCM
  digests, correlation to previous −0.06…0.03, difference RMS ≈ 0.20. Not a
  renamed loop.
- Live render: peak 0.890, RMS 0.138, dynamic range 2.10×, seam 1.2 (limit 14).
- Key/mode moved 7 times in 40 variations, tempo held 100–109 BPM. Anchored.
- Instrument registers separate cleanly (cello 36–69 … flute 72–98).

**Fixed in this pass**

- *Re-voicing could double a voice.* The index offset let two parts of one
  family land on the same seat; the live lineage carried `Reed, Reed` and a
  walk produced `Pad … Pad`. Voices are now chosen before any track is
  touched and a taken voice is skipped. Regression test:
  `re_orchestration_never_doubles_a_voice`. An *inherited* doubling (from the
  model's own score) is resolved at the first Texture movement, not before.
- *A DAW crash looked like a dismissal.* `ensure_running` set `dismissed` on
  any exit, stderr is discarded, so a crashed studio vanished silently and
  never returned until `/Music`. Now a non-zero exit is reported backstage
  and the studio reopens on the next score; only a clean exit stays closed.
- `arrangement_report` now takes a score path like the other harnesses.

**For Sol — worth doing next, in order**

1. **Density is unbounded.** The live cello plays 256/256 sixteenths (8 per
   bar, 269 ms gate) — the profile's own "an organ played as sixteenths is a
   machine gun" — and polyphony sits at 10+ voices for whole sections
   (`arrangement_report` on the live score). The arranger never thins what
   the model wrote. A per-role floor (sustaining voices ≥ eighths, bowed ≥
   quarters) enforced in `arrange.rs` would make the profile's rule
   mechanical instead of advisory.
2. **Craft is a presence checklist, so it is always 100.** Every variation
   in a 40-step walk and every `evolution_proof` line scores 100/100. It
   asks *is there* a pulse / foundation / middle / melody / cadence — and the
   mastering boundary inserts whatever is missing (`insert_performance_voice`).
   By construction it cannot fail. It has no notion of masking (two voices in
   one register), density, or the doubling above. Either add those axes or
   stop showing the number; a score that never moves is `SUGGESTIONS: 73`
   again.
3. **The cadence claim is off.** The handover says a variation every 18–36 s
   (`native_evolution_delay` clamps piece length to that). The ledger says
   median **48 s**, p10 36, p90 50 — 3 of 92 gaps land in the claimed band.
   Render + verify + swap adds ~12 s the claim doesn't count. 48 s is a good
   cadence; the document should say 48.
4. **The Organist never uses the tag.** Last 200 turns: 30 Organist replies,
   0 `[COURT_SCORE:`, 3 bare JSON, 27 prose. Composer nudges: 26 sent, 0
   tagged, 11 salvaged as bare JSON, 15 prose. Salvage carries 42 % of
   compositions; native evolution carries everything else. The profile's
   one-shot should be a *complete tagged score*, not a schema paragraph —
   this model copies shapes.
5. **Injected voices are invisible to the model.** The mastering boundary
   adds `Master Kick` / `Master Bass` at render time but the persisted score
   stays four tracks, so the Organist never sees the drums it is hearing and
   cannot write against them. Persist what was played, or tell the model.
6. Minor: `Pluck` sustain 0.10, `Bell`/`Arp` 0.06 — the audit reports them
   as "sustains" while the profile promises they decay to silence. Set to 0
   like Piano/Harp, or lengthen the release and accept the small hold.

---

## Music redesign — evolution is development, not mutation

The operator's verdict was "the music logic is terrible, and they need to
make longer looping tracks; it almost worked in V1". Measured against V1,
both halves were right, and both had one-line causes.

| | V1 | V2 before | V2 now |
|---|---|---|---|
| minimum piece | 120 s / 64 bars (`MIN_COMPOSITION_SECONDS`) | **16 bars cap** in `mastered()` — 40 s at 96 BPM | ≥ 120 s by tempo, 32–64 bars |
| hold before change | ≥ 300 s (`MUSIC_MIN_INTERVAL_SECS`) | piece length clamped 18–36 s; live median 48 s | 2 × piece length, clamped 120–600 s |
| what a change does to the notes | nothing — a new piece came from the model | rotate steps, shift degrees mod 7 | register, gate, rhythm density, chord order, voicing |
| one Motif variation, flute | — | **56 % of notes rewritten, downbeat moved** | 0 notes changed; gate 1.0 → 0.8, reed up an octave |
| one Counterpoint variation, harp | — | **88 % rewritten, downbeat moved** | 3/8 rhythm slots, no new pitches, downbeat kept |

Live after relaunch: `Harmonic Resonance`, 52 bars, **125 s** (was 40),
variation gap **255 s** (was 48). `evolution_proof` still shows distinct
digests per movement.

What changed and where:

- `Score::mastered` — `bars.clamp(8, 16)` → floor of two minutes at the
  piece's tempo, 32–64. Test: `a_mastered_piece_lasts_at_least_two_minutes`.
- `Score::evolved` — Pulse toggles one off-beat hit (never the kick, never a
  beat) and leans kick gain; Motif moves a melodic part's octave inside its
  natural range or changes its gate; Counterpoint thins or fills the middle
  voices' rhythm using neighbouring pitches only; Harmony rotates the
  progression instead of transposing the bass; Texture and Modulation
  unchanged. Tests: `a_variation_keeps_the_tune` (pitch classes and downbeat
  survive every non-Modulation movement), `every_movement_is_audible_to_the_runtime`.
- `arrange.rs` phrase development: inversion → sequence a third above the
  answer. Contour survives chord-tone snapping.
- `runtime/mod.rs`: `native_evolution_delay` 18–36 s → 120–600 s; env and
  DAW cadence clamps 12–120 → 60–900 s. `daw.py` cadences are 2 / 5 / 10 min.
- `default_score`: 48 bars, flute carries a two-bar (32-step) phrase.
- `organist.md`: asks for 32–48 bars and two-bar melodic patterns.

For Sol: `evolution_proof`'s "correlation to previous ≈ 0" was never
evidence of an audible difference. Two renders of the *same* score with
different seeds correlate near zero at sample level (humanised timing,
reverb). It proves "not byte-identical", nothing more. The tune-preservation
test above is the check that means something; a listening-scale metric
(e.g. chroma or onset-pattern similarity) would be the honest replacement.

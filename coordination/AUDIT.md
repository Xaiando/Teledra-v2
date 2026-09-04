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

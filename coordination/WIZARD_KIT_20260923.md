# Wizard Tower: from zero output to a working workshop — 23 September 2026

Target: the Wizard resident on https://teledra.dev (SSH host `administrator@85.190.99.195`,
key `D:\Teledra\tower\.ssh\teledra_wizard_tower2`). Sources are in `D:\Teledra\cloud_residents`
and `D:\Teledra\site`. Pre-change copies are in `target/wizard-useful-20260923/before/`; the
evidence is in `target/wizard-useful-20260923/evidence/`.

## What was wrong

The service ran on schedule (about 33 cycles a day) but published nothing useful. Cycles
1540-1855 (13-23 September) produced 317 attempts and one publication, and that publication
was fake:

| Work | Plan rejected | Build rejected | Published |
| --- | ---: | ---: | ---: |
| New inventions | 88 | 17 | 0 |
| Revisions of the 3 starters | 88 | 122 | 1 |

Root causes, each confirmed against the code, the logs and the live model:

1. **Silently ignored edit schema.** The server's Ollama 0.30.10 drops the whole `format`
   grammar when any string `maxLength` is about 2000 or more (1500 is enforced, 2000 is not;
   measured against the live qwen2.5-coder:3b). `REVISION_EDIT_SCHEMA` used 4000 and 6000,
   so every revision edit was unconstrained text. This caused 55% of revision build failures
   (`JSONDecodeError`).
2. **The model wrote its own test oracles.** A 3B model had to predict exact visible outputs
   for code that did not exist yet. 79 of 88 invention plans failed only because `kind` was
   generated last in the case schema, so a reset case almost never came out. The revision
   rules (`merge_revision_case`) rejected most of the rest.
3. **Gate loophole.** A behaviour case passed when any earlier setup step changed the output.
   Pattern Press r10 added a Preview button with no listener and passed because the preceding
   Next click did the work. Later candidates kept re-proposing the Preview button.
4. **Stale research.** `sorted(seen)[-500:]` kept the alphabetically last URLs, so the same 11
   Hacker News headlines recurred up to 127 times, and relevance search returned the same
   stories.
5. **Public copy overstated the work.** "Explored new ideas" was shown for plan failures, and
   the chat prompt said he "forged" tools.

The home PC's qwen2.5-coder:14b also failed 0 of 12 on the old pipeline (compute analyst),
so the pipeline design, not only model size, was blocking him.

## What changed

- **`wizard_kit.py` (new, root-owned runtime).** The model writes a constrained JSON proposal
  (sliders, numeric readouts, drawing) and then one pure `run(p)` function in a generated
  skeleton that declares every variable and includes a drawing loop. The kit renders the page:
  sliders, readout, chart or equal-scale shape, Reset, SVG/CSV export, and a method-and-code
  panel. Preflight runs in a killable Chromium worker and rejects exceptions, NaN/Infinity
  outputs, empty or flat drawings, sliders and readouts that never change, runaway loops and a
  Reset that does not restore the start. The browser contract is derived from preflight
  measurements, so the model never writes an expected value. Safe mechanical repairs
  (declaring dropped skeleton names, `const` to `let`, downsampling long series) are applied
  in code.
- **Checked science.** Inventions draw from 80 subjects, each paired with a standard, checked
  method (`SUBJECT_METHODS` in `wizard_brain.py`); the runtime's method replaces the model's.
- **Revisions** extend the Wizard's own kit apps by one slider, readout or drawing change.
  Old sliders and readouts are kept verbatim, and `revision_changes` must measure a difference.
  Starter apps are frozen showcases; the legacy path is behind `WIZARD_REVISE_STARTERS=1`, with
  its schema bugs fixed.
- **Gate v2.** A multi-step behaviour case's final interaction must itself change an asserted
  output. This rejects r10 and still passes every starter case.
- **Supervisor.** Public status uses the brain's plain note, or factual defaults. It records
  `last_attempt` and `latest_published`, and publications are written to the journal.
- **Chat.** The persona may only mention work listed in its status. The status separates the
  Wizard's own inventions from the Codex-built starters, and headlines are marked as not read.
- **Site.** The Tower labels now read "Last workshop attempt", "Latest published revision",
  "Headlines he noticed (titles only, not read)" and "Published experiments".
- **Optional stronger model.** Set `WIZARD_STRONG_OLLAMA_URL` and `WIZARD_STRONG_MODEL` on the
  unit; it is used only while it answers, and each candidate records its `model`.
- Invention cadence is `WIZARD_INVENT_EVERY` (default 4); the other cycles improve the
  Wizard's own apps.

Tests: 87 pass (`test_wizard_kit` is new and has 18 tests, some running real Chromium; the
gate suite gained an inert-final-control test; the supervisor gained status and journal tests).

## Measured on the real models (production code path, nothing published)

| Model | Inventions passing all checks | Science actually correct (manual review) |
| --- | ---: | ---: |
| Server qwen2.5-coder:3b, curated methods | 11 of 12 | about 4 of 11 |
| Home PC qwen2.5-coder:14b, same code | 9 of 10 | 9 of 9 (one unit label slip) |
| Server 3B, kit revisions | 3 of 6 | the additions invent physics |

A 3B "critic" pass was tried and rejected: it gave the right verdict on 6 of 10 known cases.
Cycle time on the server is about 1-8 minutes, well within the 1800 s budget.

The user chose to run on the server 3B only (asked 23 September). Each app therefore shows
"Automated browser checks … do not prove the science: compare the calculation with the method."
Switching to the 14b later needs a loopback tunnel from the PC and the two unit variables above.

## Deployment record

- `deploy_wizard_kit.py` waits for a running cycle, backs up, installs, checks imports as the
  resident, restarts, and rolls back on failure. The first run rolled itself back cleanly
  because its import check lacked the Wizard's home directory on the Python path. That was
  fixed, and the second run installed at 07:35 UTC after waiting 140 s for a cycle to finish.
- Backup: `/opt/teledra/wizard/backups/wizard-kit-20260923T073313Z-a38630d2`. All eight
  installed files matched the staged bytes; `teledra-wizard` and `teledra-wizard-chat` are active.
- Pattern Press revision 17 republished the verified r1 files through `wizard_supervisor.py
  --publish` under the v2 gate, withdrawing the inert Preview button.
- First live kit cycle (1867) published **Eccentricity Explorer** in about 90 s. Its closest
  approach is right, but its farthest point and drawing are wrong, which is the expected 3B
  quality.

## Open follow-ups

- Nginx sends no `Cache-Control` for HTML/JS, so returning visitors may see the old Tower wording
  for up to about a day.
- Other site pages still overstate the Wizard (for example `index.html` "working day and night",
  `inventions.html`, `fun.html`), and the "Send anonymously to the Wizard" button implies he
  reads stories.
- Grow `SUBJECT_METHODS` before it is exhausted (80 subjects, roughly 20 days at the default cadence).

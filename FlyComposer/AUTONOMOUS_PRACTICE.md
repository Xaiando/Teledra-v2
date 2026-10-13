# Local Buzzart practice

Current run: `training/buzzart/local-practice-20260917-1250`.
Dashboard: http://127.0.0.1:8791/ . The original studio and approved-track preview remain separate.

The session began on 17 September 2026 at 12:50 Oslo time and retains its original 20:50 deadline. Four workers use one numerical-library thread each, at below-normal Windows priority. There is a 4 GiB run-data limit and at most 16 listening attempts, approximately half an hour apart. It needs an awake computer; restarting the optional temporary awake helper was blocked by automatic approval review.

## What learns

Two independent lineages begin from the wide candidate and the original indie/overworld parent. Evolution strategies adjust their 1,074 engineered brain parameters. The fixed connectome coupling is preserved. Raw 61–180 second performances are judged across indie, arcade, dusk, dreamy, overworld, complex and seed-drawn grammars. Acoustic transfer is reserved for the final evaluation.

The frozen local critic measures drive, phrase continuity, independent layers, development, restraint and harmonic consistency. Its tests include endless scales, transposed loops, drones, missing bass, excessive noise and false silent gates. A smooth reward permits learning; passing every acceptance condition is a separate decision. Existing musicality, corpus-fit and within-style diversity measures are retained as additional checks.

Each lineage has 16 fixed validation seeds per context and an immutable parent baseline. A proposed checkpoint must also survive fresh three-minute paired challenges. Untouched final seeds are evaluated once at the end, within the original time budget. Higher average scores can be rejected for a loss of variety or a per-style regression.

## What is saved for listening

New-seed candidates are generated from the best checkpoint that passed validation. Dusk and overworld use explicit arrangement recipes derived from the two listener-approved pieces; other contexts remain clearly labelled raw practice captures. Unarranged and arranged scores are recorded separately, and arrangement improvements never contribute to brain fitness.

Candidates must pass score and actual audio checks before joining `listening/`. Their scores, note ledgers, stems, WAV and critic results remain available. A local pass is not listener approval and does not replace a preferred brain or publish to the main library. The approved originals remain intact.

## Controls and recovery

Use Pause, Resume and Stop on the dashboard. Stop ends the process at a safe boundary; the dashboard does not restart a stopped process. The wall-clock deadline continues while paused. Audio uses one player and never starts automatically.

Complete checkpoint bundles include both lineages, Adam state, RNG state, configuration and the original deadline. A live OS file lease prevents duplicate trainers. Resume uses saved bounds unchanged and verifies frozen inputs. Completed runs require a new run name. The hourly Codex heartbeat stays quiet during healthy progress and reports completion or a failure needing attention.

Operational repair `repairs/windows-status-sharing/manifest.json` records a bounded atomic-write retry added after Windows briefly locked `status.json` for a dashboard read. The generation-56 optimizer and RNG, critic, musical inputs and deadline were preserved exactly. The original input snapshot is retained; only this identified checkpoint-I/O module hash changed in the repaired checkpoint.

## Evidence

- `reports/autonomy-critic-smoke.json`: critic tests against real raw and approved recordings.
- `training/buzzart/autonomy-pilot-20260917-1251`: completed four-generation end-to-end pilot.
- `reports/autonomy-render-pilot`: fresh 180-second render with 1,403 verified notes.
- `tests/test_autonomy_*.py`: critic, checkpoints, Windows write retry, controls, resume and preview provenance checks.
- Current run `protocol.json`, `status.json`, `history.jsonl`, `checkpoint.json`, `jobs/`, `listening/`, and eventual `final-test.json`.

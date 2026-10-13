# Wizard Tower overhaul — 13 September 2026

Target: the actual https://teledra.dev site and its SSH-hosted Wizard resident. Sources are in `D:\Teledra\site` and `D:\Teledra\cloud_residents`; the V2 Tower crate consumes resident reports.

## Implemented

- Replaced the old generated-app gallery with an Experimental Lab: searchable collection, category filters, a separate legacy archive, clear purposes and instructions, responsive app viewer, restart, share links and revision history.
- Added three complete, credited starter experiments: Orbit Bench (gravity/orbit exploration), Pattern Press (seeded vector pattern design), Rule Garden (cellular automata and comparisons). All have actual controls, reset and SVG export. These are Codex-built starting points for the Wizard, not claimed as autonomous model output.
- Replaced throwaway generation with stable app identities and private candidate revisions. Two cycles improve existing work; every third explores a new invention direction. Existing source and acceptance cases guide improvements; unsuccessful candidates never replace the current app.
- Added independent Chromium tests on desktop and mobile. The gate checks real control outcomes, reset, rendering, console failures, layout, network access and advertised exports; publication binds the tested HTML and contract hashes.
- Preserved original creations and immutable revision history. Published reports identify the app and revision. Research discovery titles are labeled as title-only leads.
- Removed fabricated website status, chat fallback replies and story receipts. Existing chat/transcription backend routes remain intact.

## Infrastructure

- Live webroot: `/var/www/teledra.dev`; Wizard HOME: `/opt/teledra/wizard`.
- Stable runtime: `/opt/teledra/wizard-runtime`; model remains the installed `qwen2.5-coder:3b` on the 4-CPU host.
- Pinned Playwright 1.62.0. Root-owned Chromium at `/opt/teledra-wizard-browser`; narrow exact-executable AppArmor profile at `/etc/apparmor.d/teledra-wizard-chromium`. The global unprivileged-userns restriction remains enabled, and Chromium's sandbox remains enabled.
- Private Orbit Bench verification passed with the exact planned systemd restrictions, including NoNewPrivileges, PrivateTmp and ProtectSystem=strict: 8.86 seconds, 259.4 MiB peak memory. Evidence: `target/wizard-overhaul/orbit-systemd-browser-proof.json`.
- Initial deployed brain, supervisor, pages and public collection were copied into `target/wizard-overhaul/server-before`; initial deployed source hashes matched the old local copies.

## Validation and deployment record

Deployed over SSH to the live server at 06:46 UTC. A focused planner-format and Tower-status update was deployed at 06:56 UTC. The final parser, revision-edit, scheduler and public-status changes were deployed at 08:04:59 UTC; scheduled generation resumed then after the private model trials.

- Original-site rollback backup: `/opt/teledra/wizard/backups/wizard-lab-20260913T064550Z-62016528`.
- Follow-up deployment backup: `/opt/teledra/wizard/backups/wizard-lab-20260913T065611Z-f0728b7c`.
- Final runtime deployment backup: `/opt/teledra/wizard/backups/wizard-lab-20260913T080459Z-3eebe2ef`.
- All ten installed source/unit/site files matched the staged hashes. All six public site files matched their staged bytes over HTTPS. All three starter HTML/contract/report hashes and the direct-app CSP headers matched.
- `teledra-wizard`, `teledra-wizard-chat` and nginx were active; nginx configuration passed. The Wizard uses the new stable runtime, browser path and systemd restrictions.
- All 50 old manifest entries and their files survived; the UI deduplicates these into 45 Archive cards. The current manifest held 53 entries, including the three verified starter apps.
- Browser QA on the actual site passed: category filter, Archive, Pattern Press shape/symmetry/seed controls, restart restoring seed 42, mobile presentation and Details, closing removing the iframe, and the Tower's current experiment link.
- The full 53-test suite passed in 54.5 seconds using `D:\Teledra\.venv\Scripts\python.exe`. Subsequent selector/parser changes passed all 45 fast tests; the targeted-edit implementation passed its 38 brain/schema/edit tests. Together these cover 66 distinct checks, including the unchanged 14 browser-gate tests. The final 48 brain/edit/supervisor/installer tests also passed on Linux. All three starter apps passed real server browser validation during publication. No chat message or story was submitted during verification.
- Evidence: `target/wizard-overhaul/live-deployment-verification-final.json`, `orbit-systemd-browser-proof.json`, and `reset-schema-ollama-proof.json`.

The initial private real-model cycle rejected both candidates because they described unimplemented controls. Published private starter apps stayed intact. Revision planning was then narrowed to the selected app and its actual verified source; title/category/purpose stay fixed, only one feature case is proposed, and old checks cannot be silently dropped. At the six-case limit, an existing scenario can gain assertions while preserving every old action and assertion. An app whose six cases all contain ten assertions leaves the revision queue while staying public; the scheduler invents if no expandable app remains. Two new boundary tests cover this future saturation case. The latest 37 brain/supervisor/installer checks also passed on the Linux server.

The first live cycle (1539) stopped at planning after omitting an input-default reset check twice. Its result is saved in `target/wizard-overhaul/first-live-model-cycle.json`. A reset-specific schema now requires both the default input and restored output. The installed model passed an actual schema probe in 13.8 seconds (158 tokens); JSON Schema and the production contract gate both accepted it. The corrected service started a fresh cycle at 06:56 UTC. No autonomous app improvement has been claimed as accepted without an observed publication.



## Limits

Browser checks verify the recorded workflows; they are not a guarantee of creative quality or every possible interaction. No autonomous revision passed qualification in this session. The existing 3B model failed repeated planning checks, and the tested 4B alternative did not produce a working feature. Model quality remains a concrete limit; the installed service retains `qwen2.5-coder:3b`, with the new guarded revision workflow. Failed experiments stay private, with factual feedback retained for subsequent work.



## Local model evaluation

The original `qwen2.5-coder:3b` failed repeated bounded revision-planning probes even with the actual verified source and factual repair feedback: it proposed existing comparison functionality and cases whose assertions were already satisfied before the supposed new action. Native planning checks now reject these false improvements. Evidence is in `revision-planner-grounded-result.json` and `revision-planner-postcondition-review.json` under the target evidence directory.

A local `qwen3.5:4b` candidate was downloaded from the [official Ollama registry](https://ollama.com/library/qwen3.5:4b), with digest `2a654d98e6fba55d452b7043684e9b57a947e393bbffa62485a7aac05ee4eefd`. The pull verified its SHA256 and completed in 37 seconds. At a 16,384-token context the server reported 3,713,301,215 loaded bytes (about 3.46 GiB), with 2.7 GiB system memory still available. This is a measured fit, not a quality claim. Its private evaluation finished without a working candidate, so the service was not switched; the downloaded model remains available for later deliberate evaluation.

Optional `WIZARD_MODEL_THINK=false` disables separate thinking for these bounded structured/code requests, using the [documented Ollama parameter](https://docs.ollama.com/capabilities/thinking). An absent setting preserves prior API behavior. The generator does not send private code to a hosted model provider.

The first 4B planning request timed out at 180 seconds. A private repeat with a 300-second per-call bound completed in 152.4 and 72.4 seconds. The model proposed an Energy Overlay for Orbit Bench, but emitted a metadata/contract envelope instead of the requested flat plan. The first response was malformed; the second was valid JSON. Both original outcomes remain recorded as failed in `revision-planner-qwen35-repeat-result.json`.

A narrowly recognized envelope is now normalized without changing any case or inventing missing required data. Duplicate identity conflicts and a wrong selected app ID are rejected. The saved second response passed the actual parser, regression merge and contract validator after this fix: all three old cases and the model's single new feature case were preserved. This is separately recorded in `revision-planner-qwen35-normalized-result.json`; no third model planning call was made. Serializer replay confirms a structured `format` object and `think:false`; the historical probe forwarded that argument unchanged but did not retain complete request packets. Build quality remains a separate check.

Public cycle status now uses visitor-facing descriptions for research, rejection and runtime failure; exact model/browser diagnostics stay in private results, journals and logs.

Both private full-document maker attempts then hit their 480-second bounds without returning complete HTML. The first decoded 3,412 tokens at roughly 9.1 tokens/s after 98.6 seconds of prompt processing; the second reused its prompt cache but still reached 3,993 tokens before cancellation. These are throughput failures, not browser-tested code failures. The original failed build evidence and Ollama timing logs remain in the target directory.

Revision generation now uses up to eight exact source replacements rather than regenerating the whole document. All old anchors must match exactly once in the same source snapshot; overlapping, missing, ambiguous and no-op edits are rejected before anything is staged. The old/new limits are 4,000/6,000 characters, with 3,072 model output tokens. Untouched code and styles remain identical, failed browser drafts become the next repair base, and final HTML validation/publication gates remain unchanged. New inventions still generate complete compact HTML.

The final private targeted-edit call completed in 117.1 seconds, including 103.4 seconds of prompt processing and 12.9 seconds for 126 output tokens. It returned a single old/new object instead of the edits array and was rejected. Manual review found that even losslessly wrapping it would only add an inert button: no listener, overlay, energy calculation or reset behavior was supplied. No candidate was staged or published. Evidence: `maker-qwen35-targeted-result.json`, `maker-qwen35-targeted-model-response.json`, and `maker-qwen35-targeted-review.json`.

Final HTTPS/server verification passed after deployment: all ten installed files and six public site responses matched staged bytes; all three starter reports and CSP headers matched; Wizard, chat and nginx were active; all 50 legacy entries remained. The public Lab was reloaded and displayed all three verified starters. The service had begun its next cycle; that later cycle's outcome is not part of this qualification claim.

Cleanup: the temporary local SSH identity copy was removed after confirming it matched the retained original key. Its empty task-specific directory was also removed. The original SSH identity remains unchanged. The temporary local preview server had already been stopped; the browser deliverable points to the real public Lab.

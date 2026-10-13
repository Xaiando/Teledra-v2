# Fly Composer

A standalone, local experiment using real male fruit-fly connectivity to guide a homemade PyMusic composer. It renders complete pieces, compares the real graph with controls, and saves audio plus listening feedback. **Buzzart is the possible later Organist replacement; this prototype does not integrate with or control Teledra.**

## Broad listener-guided training

`train_wide.py` fine-tunes a separate candidate from the published generation-1750 Buzzart brain. It alternates six musical settings with seed-drawn grammars: different grooves, tempos, instruments, chord patterns, melodic rhythms and four source songbooks. An unstyled acoustic context is reserved for the final transfer check. It keeps the existing interpreter, instrument renderer and biological coupling fixed.

The listener practice objective measures continuity, development and interaction between roles. It respects declared chord-led, groove-led, sparse and long-phrase settings; it does not require one density, timbre or intensity curve. Existing critic, corpus-fit and within-context diversity measurements remain part of training. The objective is an engineered interpretation of the comments, not a model trained to predict the listener's taste.

Run `.\.venv\Scripts\python.exe train_wide.py --run <new-run-name>` from this folder. Defaults are 72 generations, population 32 and six worker processes. Every run needs a new directory and records frozen source/data/brain inputs, optimizer state, validation checkpoints and a final unseen-seed test. The published brain and listening library are not changed by training. Candidate selection uses validation and per-context regression limits; the final test cannot select a different checkpoint. See [the September broad-training run](training/buzzart/listener-width-20260917/report.md) for its actual outcome and paired listening samples.

## Independent performances

The **17 September composer benchmark** compares the current trained Buzzart with **Orator**, using the configured `qwen2.5-coder:14b` model. Three fresh seeds share a 40-second composition target, 96 BPM and the same five Genesis chip/FM voices. Six recordings are available as randomized A/B pairs in the listening room. All 1,371 authored notes passed an independent delivery audit; duration, repetition and layering still differ. See the [benchmark report](reports/composer-benchmark-20260917-aace4266/report.md) for the protocol, recordings and findings. This run uses Orator as requested; it does not measure the Organist persona.

To run another complete benchmark, stop the listening-room server first and run `.\.venv\Scripts\python.exe benchmark.py --persona orator` from this folder (`--persona organist` selects the other profile). The runner preserves every trial, freezes its inputs, checks the existing library for concurrent changes, and appends completed recordings without replacing earlier music or feedback. Restart the server afterward. Reusing this command repeats the same three seeds; it is replication, not a new unseen-seed test.

**Rework Organist from these notes** starts a fresh take using the listening note entered for a saved pair. Organist first chooses its own plan, authors overlapping parts on an absolute timeline, then reviews measured facts about the draft and has a bounded opportunity to revise it. The source is presented as criticized output, without its note pattern being supplied as a template. Tempo and instruments remain fixed for this study; every new note comes from the local model. The previous Organist recording and Fly reference remain saved. This is a feedback-driven authoring workflow, not model retraining or proof of improvement.

The study divides each model-planned passage into authoring windows of at most 16 quarter-note beats. Original passages and musical intentions remain intact; accepted notes from earlier windows provide context, and each request asks the composer to lay out its own supporting harmony across the whole window before adding foreground phrases. Every note remains model-authored, with page provenance and unique event IDs.

The study checks actual note coverage against the planned windows, refusing silence longer than two seconds. That threshold is an implementation choice for this fluidity experiment, not a listener-specified number or an aesthetic score. Notes can overlap an internal window ending by up to two seconds, within the overall recording limit. Invalid authoring receives at most two factual repair requests; no code fills gaps, changes pitches or stretches notes. A failed self-review retains the complete valid draft, while failed passage authoring produces no new recording. Coverage and overlap are measured on the same 48 kHz frame clock as rendering. The browser disables reworking if the active server lacks the updated workflow; restart Fly Composer to load it.

The first feedback study, **Deliberate Development**, is retained as unsuccessful evidence: its 59.275-second recording contains only 5.175 seconds of scheduled note activity, with two gaps over 20 seconds. All 109 authored notes were rendered, but that did not establish fluidity. Two subsequent attempts were refused before recording because the model did not cover its own planned windows. The format-only diagnostic returned the same short passage with plain JSON decoding; removing the numeric schema did not fix that attempt. Evidence and failures remain in `reports/organist-study-*` and `reports/organist-json-decoding-probe`.

The final separate run with 16-beat authoring windows also failed: the first eight-second window ended at three seconds after both factual repairs, leaving five seconds uncomposed. Its session report is `reports/organist-study-recording-ee233fc2ad3e455ab24ee80f870f457a/session.json`; no score or WAV was published from that attempt. The current local model has not demonstrated the requested sustained musical development. Passing adapter tests does not change that result.

**Extend both to 2 minutes** continues a selected saved pair. Fly deterministically recovers the same circuit state and performed-note memory at its last gesture, then continues. Organist receives its own prior motifs, recent notes, measured density and current position over successive local-model turns. Its instruments and tempo stay fixed during this test; new notes and musical development remain its decisions. Small response windows let it continue without serializing a whole long score in one answer.

The continuation interface uses one absolute beat timeline. Organist writes individual note objects with part, MIDI pitch, absolute beat, length and velocity; every window states its permitted beat bounds. Previous clip-based attempts remain in the evidence history. The current interface removes the need to add a local window offset to a clip placement, which caused timing mistakes in the first continued takes.

Every original authored note is retained, including timing, pitch, duration, velocity, instrument and synthesis seed. Recording-only trailing silence can contain new music, and the full extended piece is mastered again, so an identical mastered audio prefix is not promised. Earlier recordings and opinions remain saved. The two-minute duration is a shared target, not proof of compositional quality. If Organist stops early or a later response fails, valid new material is saved and explicitly labeled partial. Continuing that partial pair toward the same target reuses an already-finished Fly recording and extends only Organist.

The workbench now defaults to **Independent Fly / Organist recordings**. Each author chooses its own note sequence; both are rendered through the same homemade PyMusic instrument bank. The default capture limit is 75 seconds per performer, including a one-second release margin. Fly runs through that window; Organist may end sooner, and its recording stops after its last authored note plus the release margin (minimum file length: 10 seconds). They share neither a supplied song arrangement nor each other's notes. This is a recorded experiment, not a live stream or a controlled wiring comparison.

**Fly** runs one persistent rate-circuit simulation. Activity selects gestures, harmony movement, layer entrances and reuse of musical memory. Its note interpreter and voice-leading rules are designed code, not learned musical judgment. Its automatic instrument choices currently use 20 compatible voices across five role-specific pools, including piano and theremin.

**Organist** uses a copied Organist profile and the configured local Ollama model (`TELEDRA_MODEL`, default `qwen2.5:7b`; `TELEDRA_OLLAMA_URL`, default `http://localhost:11434`). It receives all 72 homemade instruments and seven classic timbres. It writes note clips and their placements directly, including its own tempo, pitches, rhythms, velocities and active parts. The adapter expands exactly those placements; it supplies no backing arrangement or replacement song. This is a standalone Organist persona, without the running court's conversation or production recipe machinery.

Each completed recording appears immediately. **Play recordings in sequence** uses one player; opening another recording pauses the current one. Other listening-room tabs pause through a browser broadcast. Rendering uses no audio device and can continue while saved music plays. **Download WAV** saves either recording. A failed author produces an explicit error; **Retry missing recording** keeps the successful audio and listening feedback. Previous failures remain in the session history and model evidence folder.

Independent performances pass a technical gate for valid notes, full synthesis, exact placement and unclipped output. They do not pass the older arrangement, scale-run or phrase-diversity filters. Sparse or repetitive decisions are therefore audible as authored. Fixed part gain/pan, per-note normalization and final headroom still belong to the shared renderer. Musical quality is left for listening.

The Organist's exact requests, streamed responses, profile, catalog and errors are retained in `reports/organist-*`. Its published score contains the original authoring object and inference provenance. Fly scores retain their controller trace, musical memory and decision log. Both recordings keep all note events, synthesis hashes, stereo audio and five exported stems (unused parts are silent).

The saved independent session at seed **20260914** contains **Fly: 75 seconds / 145 notes / five parts** and **Organist: 10 seconds / 123 notes / three parts**, authored by local `qwen2.5:7b`. The Organist's last note ends at 7.75 seconds. Both were published unrated. Two earlier attempts failed the interface contract; their complete evidence remains saved. The final contract bounds response size and allows an earlier authored ending. No notes were rewritten to make the published take pass.

The longer pair is saved as **Fly & Organist · continuation 3 · 120s target**: **Fly: 120 seconds / 237 notes** and **Organist: 117.625 seconds / 776 notes**. These retain every note from the original recordings. Fly's added material starts at 73.91 seconds; Organist's starts at 6.75 seconds, overlapping its earlier ending. Organist progressed through several saved checkpoints, then authored six valid passages using the absolute timeline. Its final passage exceeded the allowed window; that response and a further ending attempt were refused without clipping or replacing notes. The 117.625-second recording stays explicitly partial, and both recordings remain unrated.

An independent ledger review found no replay of the complete source in either continuation and verified every saved note against the audio audit. It also found compositional limitations: all 286 notes in Organist's last accepted continuation use 250 ms lengths and maximum velocity, and the answer part makes extreme register jumps. These are authored choices, not missing notes. Longer recordings establish sustained output, not improved musical quality.

## Development after listening

The earlier **Develop three new directions** mode remains available. It uses the preferred instrument palette from saved Keep/Reject verdicts and makes three deliberately different arrangements: spacious **Lantern Room**, lilting **Glass Tide** and syncopated **Crossing Sparks**. They share the chosen instruments and real circuit, while meter, rhythm, harmonic pacing and layer entrances vary. These sessions are musical development, not controlled wiring experiments. The original matched wiring comparison remains available under Session type.

The current folder includes this new session at seed 2026, using Luminous throughout: electric piano, theremin, cello, classic bass and vibraphone.

| Study | Musical direction | Rendered duration | Verified notes |
|---|---|---:|---:|
| Lantern Room | Spacious 4/4 dialogue, long breaths and slow harmony | 87.25 s | 135 |
| Glass Tide | Lilting 6/8, held support and a rolling upper voice | 56.86 s | 226 |
| Crossing Sparks | Syncopated 4/4, interlocking parts and a quiet breakdown | 49.89 s | 255 |

These arrangements were published unrated. Listening verdicts are saved in the library and are never inferred from rendering success. All 18 earlier track records, audio files and saved opinions were verified unchanged when this session was added.

The next listening note described all three as having potential, with Crossing Sparks a little fragmented. A targeted **connected phrasing** revision now sits beside the original in a two-version comparison. It keeps the seed, instruments, form, 255-note count and backing notes while spreading the opening, repeating related phrase heads and connecting the foreground handoffs. The source score must reproduce exactly before revision; its files and comments are preserved. Whole-piece RMS matches the original. The revised piece begins unrated.

An explicit preference for the revision, or Keep on it, can carry that phrasing into future Sparks studies. Choosing the original counts against the revision. Later explicit choices and Keep/Reject verdicts on generated revised studies update the same evidence; missing votes and unchosen directions are not rejections. Adoption is recomputed, so negative feedback or clearing evidence can withdraw it. Referencing an original in another comparison neither duplicates its library entry nor counts its existing vote twice. These are designed revision rules selected by feedback, not a learned general ability to critique music.

Preference inference groups tracks by their actual five instruments, including custom palettes. Correlated samples within one comparison contribute one averaged verdict per palette. Missing ratings contribute nothing; Clear removes that evidence on the next calculation. The initial Luminous choice is supported by one liked session, not three independent votes or an isolated proof about timbre. A preferred development direction moves earlier in the next session without marking alternatives rejected.

Defaults apply when opening the app or when an automatic development request omits instrument choices. Polling never overwrites instruments the user is editing. New tracks retain a feedback fingerprint and links to kept tracks using the same palette; original audio and opinions are preserved. This is transparent adaptation of instruments, presentation order and supported phrasing revisions, **not neural retraining or an automatic musical critic**. Free-text notes remain readable feedback; the program does not execute them or automatically translate them into musical changes.

## Open it

On this computer, double-click **FlyComposer.bat**, then open **http://127.0.0.1:8789/**. An independent Python environment is included locally in `.venv`. The server may already be running there from verification. Starting a second server on the same port will fail harmlessly; use the existing page.

For a fresh Windows installation, install Python 3.12, run **Setup.bat** once, then **FlyComposer.bat**. Setup downloads NumPy. Fly, the older composition modes and listening work offline. Organist also needs a running local Ollama service with its configured model already installed; the app does not download models. This folder can be copied separately from Teledra. The portable ZIP includes an app snapshot, synth helper, real circuit, source provenance and example library, but excludes the machine-specific Python environment and model weights. The running folder is the current development build; archive contents depend on when the ZIP was last refreshed.

The launcher prefers its own `.venv`; it can also use the existing parent development environment or Python on PATH. Playback begins only with a Play button. One shared player switches between samples. Keep records a listener preference, Reject records rejection, and Play kept pieces plays a finite sequence of kept music. New comparisons render in the background while existing music remains available.

## First listening session

The original reference sets remain available, without a preferred wiring winner:

| Seed | Ensemble | Timbres |
|---|---|---|
| 42 | Keys | Seven-voice classic PyMusic bank: keys, bell, pad, bass and pluck |
| 77 | Chamber | Piano, clarinet, violin, double bass and harp |
| 2026 | Luminous | Electric piano, theremin, cello, classic bass and vibraphone |

Each original wiring set contains randomized A/B/C labels for **real wiring**, **shuffled wiring**, and a **non-neural musical baseline**. The form, key, seed, instrument assignments, musical rules and mastering method are shared. Choose a preference and enter notes before Reveal identities. Blindness is a listening aid, not a security boundary: local score and report files contain condition names. The new named development studies all use the real controller and change their composition grammar, so a preference among them cannot establish a wiring advantage.

The workbench exposes **72 named homemade instruments plus seven classic PyMusic timbres**, including piano and theremin. Each of five parts can use any catalog voice. Presets provide practical starting combinations; some custom instrument/register combinations will work better than others.

The original wiring pieces use four sections across 32 bars. The new development studies vary sectional form, phrase lengths, meter, harmonic pace and layer entrances; seeds also change their themes, harmony and rhythms. All have five independently arranged parts, foreground rests and a composed ending. A basic structural gate rejects incomplete arrangements, silent layers, long scalar runs and insufficient phrase development. It is a coarse guard, not a musical critic or proof of quality.

## What is biological, and what is designed here?

The included graph was extracted from the official [MaleCNS Cell Type Explorer](https://reiserlab.github.io/celltype-explorer-drosophila-male-cns/), linked from [Janelia's exploration page](https://male-cns.janelia.org/explore/). It is a central-complex **population graph**: 51 named cell types representing 623 neurons, with 1,309 observed directed links totaling 847,131 synapses. Local integer IDs identify cell types, not individual neuron bodies.

The weights are exact summed connection counts from downstream table tooltips, not rounded displayed per-neuron averages. Source HTML, snapshot metadata, the selection rule, exclusions, URLs and SHA-256 digests are preserved in `data/circuit.json` and `data/type_pages/`. The cached table snapshot is dated 2026-09-09. Data are credited under CC BY 4.0; see [ATTRIBUTION.md](ATTRIBUTION.md).

The simulated units are **engineered leaky rate units**, not reconstructed living neurons. Synapse counts are scaled to coupling strengths. ACh is modeled as excitatory, GABA as inhibitory, and Glu is assumed inhibitory; receptor-specific physiology is not established by this graph. Input signals, readout projections, dynamical parameters, harmony, form and note decoding are human-designed. Aggregation loses individual-cell and bilateral structure. Unreported table edges are excluded, not proven anatomically absent. This is neither a whole-brain simulation nor evidence that flies understand music.

The real graph controls pitch variation, rhythm selection, answering phrases, texture, dynamics and some accompaniment motion. A shared seed fixes inputs/readouts, motif, key and form. The shuffled control preserves directed in/out degrees, source-associated raw weights and signs, and all 51 within-type links. It swaps only inter-type destinations. Original graph normalization is shared; destination weight totals and resulting dynamics can change, so this null does not isolate topology from all weighted-strength effects. The baseline uses deterministic sinusoidal controls over the same decoder range. A disconnected-graph score is also generated to check that recurrent wiring changes the actual notes after decoding.

The original wiring comparisons do not train neural parameters. **Buzzart now trains engineered input/readout projections, context weights, bias and bounded dynamics against designed musical measurements**, while biological coupling stays fixed. Saved comments can guide explicitly recorded training objectives and revisions; saving a comment alone does not retrain a brain. Musical judgment still comes from listening. Replacing Organist requires listener preference across multiple pieces and later integration work.

## Audio and persistence

The renderer uses explicit sample placement, with no live note scheduler to skip attacks. Every scheduled note must return its full frame count, finite nonzero audio, and an in-range destination; otherwise the candidate is refused. Native patch random seeds are derived from musical identity rather than insertion order, keeping identical notes' timbres identical across controls. Five stems and stereo output are retained. The older matched comparisons share whole-piece RMS. Independent recordings use the same nominal RMS target, reduced separately if needed for stereo and stem headroom. This is not perceptual loudness matching; extremely quiet authored notes can remain inaudible.

`library/index.json` records comparisons and listener verdicts. Each track directory contains `audio.wav`, five named stem WAVs, `score.json` and `audit.json`. Index updates use a temporary file and atomic replacement before changing live state. A failed candidate does not replace saved music. Closing during rendering can leave unindexed candidate folders, which are ignored on restart. Only one composition worker is allowed per running server. Do not run multiple servers against the same library concurrently.

The browser server binds to loopback only. Independent Organist authoring calls the configured local Ollama endpoint. Rendering does not use an audio device, start the court, modify AppData, or download data. The packaged Windows helper is the existing homemade instrument renderer invoked solely with `--music-note-server`.

## Reproduce and verify

From this directory:

```powershell
.\.venv\Scripts\python.exe run.py --check
.\.venv\Scripts\python.exe run.py --perform 20260914 --seconds 75
.\.venv\Scripts\python.exe run.py --develop 2026
.\.venv\Scripts\python.exe run.py --compose 42 --ensemble keys
.\.venv\Scripts\python.exe audit_library.py
```

`--perform` records both independent authors; `--develop` saves three contrasting studies using the learned starting palette; `--compose` saves the original matched blind wiring set. None plays audio. Use the browser while its server is running; CLI rendering should run with the server closed so two processes cannot write the same library. Scores retain author identity, note ledgers and PCM hashes; circuit authors also retain activity traces. `audit_library.py` reads actual saved WAVs, checks frame counts, exported-stem reconstruction, RMS, peak headroom, silent release tails, distinct comparison PCM and identical synthesis for shared musical notes of the same duration. Independent sessions allow one successful author when the other fails, variable active parts and different final RMS; completed-author statuses must match their saved recordings.

Developer checks require the optional data parser dependencies:

```powershell
.\.venv\Scripts\python.exe -m pip install -r requirements-data.txt
.\.venv\Scripts\python.exe -m unittest discover -s tests -v
```

Automated tests cover data provenance and graph controls, all 72 native voices, exact note placement, invalid audio refusal, persistence, worker and HTTP behavior, feedback adaptation, musical directions and source preservation. Independent-session checks include faithful clip expansion, model response bounds, earlier authored endings, sparse/overlapping notes, retry isolation, stem headroom and concurrent feedback. The 29 focused continuation tests in `reports/extended-final-tests.txt` also verify absolute timing, legacy recording resumption, exact source-note preservation, recovered Fly state, partial failures and reuse of an already-completed recording. The 22 focused study tests in `reports/organist-study-final-tests.txt` cover literal feedback, source preservation, model-authored revisions, silence checks, window-scoped frame metrics, faithful multi-window note placement and transactional publication failures. Earlier full-suite logs remain in `reports/extended-tests.txt` and `reports/independent-tests.txt`. Mock-model tests establish adapter behavior; the published Organist recordings separately retain actual local-model inference evidence.

The latest `reports/audio-validation.json` reports **29 saved pieces and 8,650 verified note placements**, including the first unsuccessful feedback study. All 28 tracks, 13 comparisons, full audio hashes and listening verdicts/comments that existed before the feedback studies were verified unchanged in `reports/organist-study-preservation.json`. The earlier extension audit remains in `reports/extended-preservation.json`. These checks establish technical behavior and differences in the saved PCM; they do not establish musical quality or a preferred performer.

To rebuild the selected graph from its preserved official pages, install `requirements-data.txt` and run `python -m flycomposer.connectome`. Existing cached pages are reused. Preserve the original snapshot if comparing experiments; source changes can produce a different graph.

No Cargo build is required to launch this fork. The included native note helper was copied from the existing compiled homemade renderer and verified directly. Its fingerprint is in [ATTRIBUTION.md](ATTRIBUTION.md).

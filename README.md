# Teledra V2

For the 14 October 2026 source snapshot and Gemini Argon inspection, start with
[the review handoff](coordination/GEMINI_ARGON_REVIEW.md) and
[current validation](coordination/ARGON_VALIDATION.md).

Teledra V2 is the modular rewrite of the sovereign court. It preserves the
operator-facing capabilities of `D:\Teledra`, while fixing ownership,
boundedness, background execution, process cleanup, and dead code. The only
language model is local Ollama `qwen2.5:7b`.

The old repository remains a read-only behavioral reference and rollback. V2
does not import executable code from it. When a V2 outreach or Restream config
does not exist yet, it can read the operator-owned V1 credential config in
place so the migration does not expose or duplicate secrets.

## Architecture

| Folder | Owner |
| --- | --- |
| `Agents` | Small, independently editable court-member profiles |
| `Court` | Role-local sessions and typed hidden action tags |
| `Teledra/Brain` | Bounded Ollama request/response boundary |
| `Teledra` | TUI, event loop, environment report, and runtime coordination |
| `Teledra/src/runtime` | Separate command, action, artifact, and sidecar owners |
| `TTS` | Warm LuxTTS worker, reference voices, device-routed playback |
| `Music` | Native score schema/synthesis/playback plus the hot-reload Organist DAW |
| `Fractus` | Bounded scene renderer plus the animated, hot-reload Fractus Studio |
| `Memory` | Durable transcript recall and dream consolidation |
| `Research` | Public-web research with provenance and response limits |
| `Sidecars` | Supervised optional mic, vision, stream, outreach, treasury, MCP, and YouTube adapters |
| `Swarm` | Durable root-court missions and evidence-bound completion |
| `Frankenswarm` | Provenance composition plus standalone merge laboratory, blackboard/API, evaluation, and sandbox |
| `Kraken` | Bounded verification plus standalone durable taskforce, research, forge, Hub, and game graduation |
| `Tower` | Path-safe resident report inbox |

## Run

Requirements:

- Ollama running locally with `qwen2.5:7b` installed.
- Rust 1.95 and the Visual Studio C++ Build Tools workload on Windows.
- Optional Python dependencies for speech and device/network sidecars.
- Tesseract OCR for text extraction from `/Vision` captures.

Provision the repository-local Python environment once with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Setup-Python.ps1
```

The launcher automatically selects `.venv\Scripts\python.exe` afterward.
Setup also pre-caches the pinned LuxTTS and Whisper model snapshots so the
first court launch does not spend its bounded TTS startup window downloading
weights.

```powershell
.\Teledra.bat
```

Start the court from its shortcut or `Teledra.bat`. The launcher rebuilds first,
so a source change is never left behind in a stale executable. Starting the
executable from a script or scheduler instead can leave the terminal without
24-bit colour, which renders the whole court white on black.

Normal launch keeps unavailable optional integrations disabled and reports
them in Private Court Activity. Use `--strict` to require every capability or
`--check-environment` to print a full report.

## TUI

The wide court mirrors the original composition: portrait, cognitive and
somatic panels, shared neural channel, backstage event trace, chat input, and
a separate YouTube ingest input. Direct operator speech always enters the
Teledra/Queen channel; delegated court replies remain visible in the same
public feed.

```text
Enter             submit the focused input
Left/Up           focus chat input
Right/Down        focus YouTube ingest
Tab / Shift+Tab   cycle performance mode
Shift+Up/Down     scroll one line
PageUp/PageDown   scroll five lines
Ctrl+M            mute/unmute Court Synth (the piece keeps running)
Ctrl+U            evolve/play the current music
Ctrl+L/K/E/P      like/dislike/expand/playlist
Ctrl+G / Ctrl+J   refresh vision / toggle continuous microphone
F1 or ?           help
Esc or Ctrl+Q     exit
```

Type `/Vertical` to open a second, read-only portrait TUI for a vertical OBS
source. It mirrors the same live session while the normal TUI remains open;
it never starts another model, speech worker, or autonomous court. On Windows
it opens in a dedicated 54x48 portrait console so Windows Terminal cannot
stretch it into a landscape tab. Escape or Ctrl+Q closes only that view.

`/Dashboard` and `/Work` likewise open dedicated read-only consoles driven by
the same atomic session snapshot. Dashboard separates operational status,
private diagnostics, and court activity; Work restores the YOU/AGENT/BOUNTY/
IDEA lanes alongside durable missions and tasks. Closing a view never stops
the court.

## Commands

| Area | Commands |
| --- | --- |
| Presentation | `/Vertical`, `/Rundown`, `/Say [voice] <words>` (speaks your exact words in the Queen's or a named court voice), `/VoiceDesk [on|off]` (typed lines are spoken verbatim and nothing runs on its own; start in it with `TELEDRA_VOICE_DESK=1`), `/Lock <topic>`, `/Podcast [60m..240m] <topic>`, `/Podcast status`, `/Unlock` |
| Modes | `/Mode <normal|comedic|empathetic|dark|babble|streamer|copilot|night>`, `/Broadcast`, `/NightDesk`, `/CoPilot` |
| Music | `/Music` resumes the selected engine; `/PyMusic` and `/Synth` switch exclusively; `/MusicCompare` toggles automatic alternation; `/MusicOff`, `/MusicToggle`, `/Keep [note]`, `/MusicReject [note]`; Ctrl+U requests the next composition |
| Visuals | `/Art [DSL]` for a living still; `/Fractus [DSL]` or `/FractusLive [DSL]` for an animated loop |
| Memory/research | `/Dream`, `/Reflect` (behavior audit), `/Research <query|URL>`, `/Links`, `/Study [query]`, `/YouTube <URL>` |
| Inputs | `/Mic`, `/Vision`, `/Somatic`, `/Stream`; paste a Restream embed link or a Social Stream Ninja dock link (or `/ssn <session ID>`) to connect live chat. The key is saved in the data folder and reconnects on every launch. Social Stream needs "Enable remote API control" and "Send chat messages to API server" on under Global settings → Mechanics, and a fixed session ID under Session Options |
| External | `/Diplomat [message]`, `/Agents`, `/Outreach`, `/Treasury` (ledger), `/Scout` (refresh), `/Mcp <server> [method] [JSON]` |
| Work | `/Mission <objective>`, bare `/Mission` (status), `/MissionCancel`, `/Work`, `/Tasks`, `/Goals`, `/Innovate`, `/Tower` |
| Governance | `/Proposals`, `/Approve <id|all>`, `/Reject <id|all>` |
| Operations | `/Dashboard`, `/Verify`, `/Workshop` |
| Off-air test | `/Test`, `/TestStatus`, `/TestKnobs`, `/TestCast`, `/TestScene`, `/TestSilence`, `/TestRate`, `/SimChat`, `/TestTick`, `/TestMusic` |

Original aliases such as `/PyMusic`, `/FindWork`, `/Suggestions`,
`/ApproveAll`, `/WizardPull`, `/MemoryPolicy`, and `/Sleep` remain accepted.

### Podcast sessions and shared videos

Paste a YouTube video link directly into chat (optionally with a question),
into the YouTube field, or use `/YouTube <URL>`. Teledra responds using retrieved
captions, including non-English captions when English is unavailable. If
captions cannot be retrieved, she reports that limitation and uses only any
available title/channel metadata. This does not watch the video's images or
listen to its audio. Long transcripts are sampled across the recording.

`/Lock <topic>` starts Podcast mode with a 60-minute plan. For a longer show,
use `/Podcast 90m <topic>` (60–240 minutes). Teledra and the court develop twelve
timed angles with longer spoken contributions and bounded episode notes.
The topic stays locked after the planned duration until you use `/Unlock`;
model-generated topic, unlock, and stop tags cannot end the operator's lock.
`/Podcast status` or `/Rundown` shows progress. `/Mode normal` pauses the show
and its clock; bare `/Podcast` resumes it. `/Unlock` ends it and restores the
mode used before the episode. Ordinary questions can interrupt the conversation
without changing the locked subject. Sessions and episode notes last for the
current app run; restarting starts fresh.

Explicit Podcast mode schedules speech even when ordinary idle autonomy is
disabled, while still respecting off-air test mode and the speech queue. Actual
delivery depends on the local model and TTS; the timing plan does not guarantee
an uninterrupted hour of audible speech if either service is unavailable.

## Configuration

| Variable | Meaning |
| --- | --- |
| `TELEDRA_OLLAMA_URL` | Non-default Ollama endpoint; the model remains fixed |
| `TELEDRA_DATA_DIR` | Durable runtime data root |
| `TELEDRA_AUTONOMOUS_INTERVAL_SECS` | Court cadence; `0` disables it |
| `TELEDRA_DISABLE_TTS` | Disable speech when set |
| `TELEDRA_MUSIC_VARIATION_SECS` | Verified variation cadence, `60..900` seconds; default holds two passes of the piece, bounded to `120..600` seconds |
| `TELEDRA_MUSIC_EVOLVE_SECS` | Separate model-composer creative nudge cadence; default `300`, `0` disables |
| `TELEDRA_DISABLE_MUSIC` | Disable automatic Organist synthesis/playback at startup when set |
| `TELEDRA_FRACTUS_EVOLVE_SECS` | Variation within the current animated composition; default `150`, minimum `60` seconds |
| `TELEDRA_FRACTUS_EXPLORE_SECS` | Artist research and invention cadence; default `180`, minimum `120` seconds |
| `TELEDRA_DISABLE_FRACTUS` | Disable automatic Fractus gallery startup/evolution when set |
| `TELEDRA_PYTHON` | Python used for TTS and sidecars |
| `TELEDRA_LUXTTS` | Optional LuxTTS source checkout |
| `TELEDRA_AUDIO_DEVICE` | Full or partial output device name, useful for OBS routing |
| `TELEDRA_TTS_SPEED` | Speech pace from `0.55` to `1.10`; default `0.90`, lower is slower |
| `TELEDRA_QUEEN_VOICE` | Teledra's reference recording: `custom` (default), `clean`, `sarcastic`, `analytical` |
| `TELEDRA_PORTRAIT_DENSITY` | `half` (V1-compatible default, 2 pixels per cell), `quadrant` (4), or `sextant` (6) |
| `TELEDRA_TESSERACT` | Optional path to a non-standard Tesseract OCR executable |
| `TELEDRA_STREAM_CHAT_URL` | Restream-compatible JSON message endpoint |
| `TELEDRA_OUTREACH_URL` / `TELEDRA_OUTREACH_TOKEN` | Explicit approved diplomacy endpoint |
| `TELEDRA_MOLTBOOK_CONFIG` / `TELEDRA_MOLTBOOK_API_KEY` | Optional Moltbook config override or API key |
| `TELEDRA_OUTREACH_CHANNELS_CONFIG` | Optional approved webhook-channel config |
| `TELEDRA_TREASURY_URLS` | Comma-separated read-only opportunity feeds |
| `TELEDRA_MCP_CONFIG` | Approved MCP server configuration |

Model actions that write files are confined to the V2 data directory.
Diplomacy generated by the model is held as a pending proposal and is not
sent until `/Approve` records the decision.

`/Help` opens grouped command help. Ordinary question marks remain editable
in chat. Pending proposals expire after seven days (Moltbook actions after
48 hours); expired records remain on disk. Environment and portrait checks
do not terminate an active court. The launcher stops only this checkout's
executable and refuses to launch a stale build after compilation fails.

See [the September overhaul](coordination/ASTRA_OVERHAUL.md) and the subsequent
[operational acceptance](coordination/OPERATIONAL_ACCEPTANCE.md) for the music,
agent, speech, launcher, and live process checks. `/MusicMute` also works where
the terminal encodes Ctrl+M as Enter.

## Verify

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p teledra -- --check-environment
```

Start from [coordination/HANDOVER.md](coordination/HANDOVER.md) if you are
picking this up. See [MIGRATION.md](MIGRATION.md) and
[coordination/PARITY.md](coordination/PARITY.md) for ownership and the live
replacement gate, and
[coordination/PARITY_GAPS.md](coordination/PARITY_GAPS.md) for deliberate safety
boundaries and remaining live hardware checks. The execution-path comparison
for the Fractus and Organist apps is recorded in
[coordination/FRACTUS_ORGANIST_AUDIT.md](coordination/FRACTUS_ORGANIST_AUDIT.md).

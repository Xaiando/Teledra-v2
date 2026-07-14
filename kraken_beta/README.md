# Kraken — operator guide

The kingdom's silent research & productivity mode. Drop jobs in, get verified,
cited work out of `vault/`. All local models (qwen / Ornith / moondream), zero
frontier tokens at runtime. Design: [SPEC.md](SPEC.md).

## Quick start

```powershell
cd D:\Teledra\kraken
D:\Teledra\.venv\Scripts\python.exe kraken.py add research_local "How does the treasury income policy work?"
D:\Teledra\.venv\Scripts\python.exe kraken.py run
D:\Teledra\.venv\Scripts\python.exe kraken.py status
```

The report lands in `vault/<job-id>-report.md` with a Sources section.

## Commands

| Command | What |
|---|---|
| `kraken.py add <skill> "<input>"` | queue a job |
| `kraken.py run [N]` | process up to N jobs now (default 25) |
| `kraken.py daemon` | silent loop — polls every 20 s, survives Ollama restarts |
| `kraken.py worker [name]` | **taskforce mode**: hub-connected worker (see below) |
| `kraken.py status` | queue counts + recent jobs + journal tail |
| `kraken.py graduate-games` | probe the stable production game inventory and write a per-game acceptance manifest |
| `kraken.py skills` | list installed skills |

`graduate-games` is read-only with respect to game artifacts. It excludes
`captain_comic_clone`, resolves each game from `game_inventory.json` plus any
agreeing local `.kraken-game.json`, verifies the runtime adapter's actual
profile/version, and writes `output/game_acceptance_manifest.json`. A game that
changes during its probe is rejected and must be rerun on a stable snapshot.

## Skills

Run `kraken.py skills` for the live list:

- `research_local` — cited answers from `knowledge/` + root docs; a per-source
  qwen relevance gate drops tangents, and if nothing local answers, it
  **escalates to `research_web` automatically** (local-first, web second)
- `research_web` / `research_synth` — DDG (Bing fallback) search, real fetched
  sources only; honest failure (requeue) when fewer than 3 sources land
- `research_fanout` — recursion demo: decomposes a broad question, delegates
  to `research_local` children, then a join step merges them into one report
- `code_forge` — Ornith writes & repairs code until `verify_code` passes.
  **Retrieval-augmented**: recalls the most relevant past lessons from
  `lessons/code_forge_lessons.jsonl` before forging, so it stops repeating
  mistakes it has already paid for (the flywheel).
- `coding_mcp` — safe coding-tool surface for workers: tree/read/search,
  Python compile/test checks, and git status without raw shell access
- `prod_digest` / `prod_vault` — daily briefs + evergreen distillation
- `introspect` — the taskforce audits its OWN history: reads every journal
  verdict + forge lesson and emits a ranked, evidence-backed improvement backlog
  to the vault. `kraken.py add introspect all` (or a skill name to focus).

### Self-improvement & safety net

- **Flywheel**: forge → fail → repair → log (`lessons/`) → **recall**
  (`kernel/recall.py`) → forge better.
- **Self-audit**: `introspect` turns operational history into a prioritized
  backlog — the system points at its own next improvement.
- **Regression suite** — the net that makes continued self-improvement safe:
  ```powershell
  D:\Teledra\.venv\Scripts\python.exe kraken\tests\test_regression.py
  ```
  Fast, offline, deterministic. Green = every earned safety property still holds
  (ACE guard, path allowlist, padding rejection, workspace confinement, orphan
  reaper, anti-gaming, escalation guard, recall). **Run it before shipping any
  edit** — a change that breaks a hardening is caught here, not in production.

### Code forging (code_forge)

```powershell
D:\Teledra\.venv\Scripts\python.exe kraken.py add code_forge "{\"task\": \"Write slugify(text) that lowercases and hyphenates\", \"filename\": \"slugify.py\", \"tests\": \"from slugify import slugify\nassert slugify('Hello World') == 'hello-world'\"}"
D:\Teledra\.venv\Scripts\python.exe kraken.py run
```

For JSON payloads in PowerShell, prefer a file so quotes and newlines survive:

```powershell
D:\Teledra\.venv\Scripts\python.exe kraken.py add code_forge --input-file jobs\payload.json
# or:
D:\Teledra\.venv\Scripts\python.exe kraken.py add code_forge @jobs\payload.json
```

Ornith drafts the module, `verify_code` runs py_compile + the tests, and
failures loop back for up to 3 repair attempts. Output lands in the job's
workdir (path shown by `status`).

Workspace builds are transactional: Kraken generates and repairs in the job
workdir, then publishes into `workspace/` only after the verifier passes. For a
senior-reviewed artifact, use `"verify_only": true`; browser jobs with
`"quality": "beast"` must additionally pass the headless Play/movement/jump/
audio/damage/progression probe before they can be marked done.

### Coding tools (coding_mcp)

`coding_mcp` gives the taskforce practical coding MCP-style tools while keeping
paths confined to Kraken root and `workspace/`.

```powershell
D:\Teledra\.venv\Scripts\python.exe kraken.py add coding_mcp "{\"op\":\"tree\",\"path\":\".\",\"max_files\":80}"
D:\Teledra\.venv\Scripts\python.exe kraken.py add coding_mcp "{\"op\":\"search\",\"path\":\".\",\"pattern\":\"TODO\"}"
D:\Teledra\.venv\Scripts\python.exe kraken.py add coding_mcp "{\"op\":\"py_compile\",\"path\":\"games/animated_game\"}"
```

Supported ops: `tree`, `read`, `search`, `py_compile`, `run_tests`,
`git_status`. In Mission Chat, plain requests like "inspect the project",
"search for TODO", "compile the animated game", or "git status" route here.

### Productivity (prod_digest / prod_vault)

Digest logs or notes into a daily brief:

```powershell
D:\Teledra\.venv\Scripts\python.exe kraken.py add prod_digest "D:\Teledra\logs max_files=2 max_lines=20"
D:\Teledra\.venv\Scripts\python.exe kraken.py run
```

Output: `vault/<job-id>-digest.md` with `## Sources` listing every file read.
`verify_digest` enforces coverage (each eligible file cited or waived).

Distill a finished vault report into an evergreen note:

```powershell
D:\Teledra\.venv\Scripts\python.exe kraken.py add prod_vault "vault/k-20260707-46d54b-report.md"
D:\Teledra\.venv\Scripts\python.exe kraken.py run
```

Output: `vault/<report-stem>-evergreen.md` with `## Evergreen`, `## Key Facts`,
`## Source`.

## Taskforce mode (the mode hub)

Kraken has its own Agent Hub instance, separate from the main command center:

- **One-click launch**: double-click `kraken_taskforce.bat` — starts the mode
  hub, both workers, and a desktop chat room wired to the mode state.
- **Mode hub**: `http://127.0.0.1:3838`, state in `hub/data/hub_state.json`
  (that layout lets the desktop app read it via its cwd-relative default).
- **Workers**: `kraken.py worker kraken-worker-1` (repeat with new names for
  more). Each registers on the mode hub, polls Mission Chat, and signals every
  job outcome. Mission intake is dedicated-claim: a shared lock guarantees
  exactly one worker queues each mission.
- **Drive it from Mission Chat**: type `<skill>: <input>` in the mode hub's
  chat (desktop app pointed at 3838, or POST /api/messages) — e.g.
  `research_local: what does the swarm design require?` A worker queues it,
  grinds it, and posts the vault path back as an agent signal.
  Plain requests are also translated best-effort: "make/write/build/create..."
  becomes `code_forge`, ordinary questions become `research_local`, and
  digest/distill wording routes to the productivity skills. Explicit
  `<skill>:` lines still win when you want exact control.
  Game requests now default to a browser/canvas `index.html` stress lane with
  animation-loop, input, HUD, win/lose, and no-external-asset verifier checks.
  The verifier also rejects a common dead-button bug: inline `onclick` handlers
  that call functions hidden inside a JavaScript closure.
  New browser-game prompts also request Web Audio API sound effects and guard
  against instant win/lose states immediately after pressing Play.
  Ask for "terminal" or "console" when you specifically want a Python terminal
  game.
- **Watch status in Tasks**: every Kraken job is mirrored into the Hub Tasks
  panel. `In Progress` means queued/running, `Done` means the output path is
  ready, and `Blocked` means the job failed or exhausted attempts. Agent Signal
  is the live work stream: workers post `working:` updates as skills log
  progress, verifier failures, repair attempts, and final verdicts. Completed
  Python artifacts include `Run:` commands.
- **Workspace = free-rein zone**: workers share `workspace/` and may build
  anything inside it — multi-file projects, subfolders, all of it. Point the
  taskforce at any folder: `kraken_taskforce.bat D:\SomeProject` (or
  `kraken.py worker <name> <folder>` / `KRAKEN_WORKSPACE`). Skills receive it
  as `ctx["workspace"]`. `code_forge` accepts `"dir": "subfolder"` in its JSON
  to build there, e.g.
  `code_forge: {"task": "...", "filename": "app.py", "dir": "myapp", "tests": "..."}`
  — multi-line tests typed in chat are fine.
- Senior agents (Claude/Codex/Antigravity/Grok) can also register on 3838 to
  direct workers or review outputs without touching the main command center.

## How it stays safe

Bounded recursion (depth 3, 5 children/job, 2 attempts), skills confined to
their workdir + `vault/`, every cycle journaled to `journal/YYYYMMDD.jsonl`,
verifier gate before any job counts as done. The daemon adds per-job timeouts
via `kernel/supervisor.py` (Codex's lane).

## Troubleshooting

- **"ollama is not reachable"** — start Ollama; the daemon just waits it out.
- **Job `blocked`** — unknown skill or exhausted attempts; `feedback` field in
  `jobs/jobs.jsonl` says why. Re-queue by adding it again.
- **Queue file grows forever** — updates are append-only by design; run
  `python -c "from kraken.kernel.queue import Queue; Queue(r'D:\Teledra\kraken').compact()"`.

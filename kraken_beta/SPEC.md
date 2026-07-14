# KRAKEN — the kingdom's silent research & productivity mode

> Mission (from the crown, via Agent Hub 2026-07-07): "a recursive useful environment
> with skills and harnesses… the best of all of you combined as a local work horse and
> research kraken… a new silent work assistant mode. This is a Teledra fork project."
>
> Orchestrator: **Claude**. Builders: **Codex, Antigravity, Grok, Claude**.
> Coordination: Agent Hub (`http://127.0.0.1:3737`) — claim your task, post signals.

## 1. What it is

A local, silent, recursive work loop. The operator (or the court) drops **jobs** into a
queue; Kraken grinds them with **skills** (modular capabilities) checked by
**harnesses** (mechanical + model verifiers), journaling everything, and depositing
finished work in a **vault**. Jobs may spawn bounded child jobs — that's the recursion:
research fans out into sub-questions, a build fans out into build+test+doc.

All local models, zero frontier tokens at runtime:

| Model | Role |
|---|---|
| `qwen2.5:7b` | planning, reasoning, synthesis, self-check |
| `hf.co/deepreinforce-ai/Ornith-1.0-9B-GGUF:Q4_K_M` | code generation (the subconscious — see `../CODING_SUBCONSCIOUS.md`) |
| `moondream` | vision (screenshots, images in research) |

## 2. Layout & file ownership (DO NOT cross lanes without a hub signal)

```
kraken/
  SPEC.md               <- Claude (this file; interface changes go through Claude)
  kraken.py             <- Claude (CLI: add | run | daemon | status)
  kernel/               <- Claude scaffolded; Codex hardens (see Task C)
    llm.py                 Ollama client (generate/chat, timeouts, retries)
    queue.py               file-backed job queue (jobs/jobs.jsonl, atomic claim)
    skills.py              skill registry/loader
    harness.py             verifier dispatch
    loop.py                the recursive worker loop
    supervisor.py          <- Codex: Wizard-pattern supervisor (timeouts, auto-revert,
                              progress-or-reject, max cycles/day)
  skills/
    research_local/     <- Claude (demo: answer from knowledge/, cited)
    research_web/       <- Antigravity (search+fetch+extract, robots-respecting)
    research_synth/     <- Antigravity (multi-source synthesis w/ citations)
    code_forge/         <- Codex (Ornith gen -> verify -> repair loop)
    prod_digest/        <- Grok (digest logs/folders/notes into a daily brief)
    prod_vault/         <- Grok (distill outputs into evergreen vault notes)
  harness/
    verify_research.py  <- Antigravity (citations exist, claims spot-checked by qwen)
    verify_code.py      <- Codex (py_compile / pytest / cargo check as applicable)
    verify_digest.py    <- Grok (coverage: every input source represented or waived)
  vault/                   finished reports & notes (output only)
  journal/                 append-only run journals, one .jsonl per day
  jobs/                    queue state (jobs.jsonl) + per-job workdirs
```

## 3. Interfaces (the contract — build to these)

**Job** (one JSON object per line in `jobs/jobs.jsonl`; queue.py owns the file):
```json
{"id": "k-20260707-a1b2", "skill": "research_local", "input": "question or payload",
 "status": "queued|running|done|failed|blocked", "parent": null, "depth": 0,
 "attempts": 0, "feedback": [], "created": "...", "updated": "..."}
```

**Skill** = directory under `skills/` with:
- `SKILL.md` — what it does, inputs, outputs, model budget (frontmatter: `name`, `timeout_s`, `max_children`)
- `run.py` exposing `def execute(job: dict, ctx: dict) -> dict`
  - `ctx` = `{"llm": llm module, "root": kraken dir, "workdir": jobs/<id>/, "log": fn}`
  - returns `{"ok": bool, "output": "<path under vault/ or workdir>", "notes": "...",
     "children": [{"skill": "...", "input": "..."}]}`  (children optional)

**Harness**: `harness/verify_<domain>.py` exposing `def verify(job, result, ctx) -> dict`
  - returns `{"passed": bool, "reasons": [".."]}`. Mechanical checks first; qwen
    spot-check second. A skill names its harness in SKILL.md frontmatter (`harness:`).

**Loop invariants (safety rails — inherited from the Wizard/tower):**
1. `MAX_DEPTH = 3`, `MAX_CHILDREN = 5` per job, `MAX_ATTEMPTS = 2` (then `blocked`).
2. Skills write ONLY inside their `workdir`, `vault/`, and the operator-designated
   **workspace** (`ctx["workspace"]` — default `kraken/workspace/`, overridable via
   `KRAKEN_WORKSPACE` or a worker launch arg). The workspace is the free-rein zone:
   multi-file projects, subfolders, anything — it belongs to the taskforce. Never
   write elsewhere, never `knowledge/` state files, never the network except
   `research_web`.
3. Every cycle journaled (`journal/YYYYMMDD.jsonl`): job id, skill, ms, verdict, notes.
4. Failed verify => feedback appended to job, requeued (until MAX_ATTEMPTS).
5. Daemon mode: per-job wall-clock timeout (skill's `timeout_s`, default 300).

## 4. Task board (mirrored in Agent Hub — claim there before starting)

| Task | Owner | Acceptance |
|---|---|---|
| A. Spec + kernel scaffold + demo end-to-end | Claude | `kraken.py add` + `run` completes a `research_local` job: cited report in vault, journal entry, harness pass |
| B. Web research skills + verifier | Antigravity | `research_web` + `research_synth`: given a question, fetches >=3 sources, synthesizes cited report; `verify_research` catches a fabricated citation |
| C. Supervisor + code_forge + verifier | Codex | `supervisor.py` survives a deliberately hung skill (timeout kill + journal); `code_forge` writes+repairs a small module until `verify_code` passes |
| D. Productivity skills + verifier | Grok | `prod_digest` turns a folder of notes/logs into a brief; `prod_vault` distills a finished report into an evergreen note; `verify_digest` enforces coverage |
| E. Integration review + operator docs | Claude | all skills runnable from CLI, README for the operator, final hub report |

## 5. Working agreement

- **UI rule (hard, from the operator):** never make the operator use a web
  browser to interact with a tool. Native desktop UI or CLI only; browser UIs
  only for actual websites/web-specific deliverables. The mode hub's HTTP API
  is agent plumbing, never a human surface.
- Python 3.12, stdlib + `D:\Teledra\.venv` (requests etc. available). Run with
  `D:\Teledra\.venv\Scripts\python.exe`.
- Match kernel idioms; no new deps without a hub signal.
- Small commits… of files. Repo-wise this is untracked-until-operator-commits, like the
  rest of the kingdom. Do NOT `git commit` in D:\Teledra without the operator.
- When your task hits acceptance: set your hub task to `done` and post an
  `agent_signal` with evidence (paths + what you ran). Claude reviews and integrates.
- Blocked? Post `agent_signal` to `claude`, mark hub task `blocked`, keep the reason
  in the message.

*The court dreams it, the kraken grinds it, the seniors judge it, the crown merges it.*

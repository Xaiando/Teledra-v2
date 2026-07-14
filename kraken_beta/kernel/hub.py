"""Kraken mode-hub bridge — workers appear as agents on their own Agent Hub.

The taskforce mode runs a dedicated agent-hub instance (default
http://127.0.0.1:3838, state in kraken/hub/hub_state.json) separate from the
operator's main command center on 3737. Workers register presence, post
lifecycle signals per job, and the operator (or senior agents) can watch and
direct the mode through the same desktop/API surface as the main hub.

Everything here is best-effort: the loop must keep grinding when the hub is
down, so every call swallows network errors and returns None.
"""

from __future__ import annotations

import json
import os
import re
import urllib.error
import urllib.request

try:
    from kraken.kernel import game_prompts
except Exception:
    game_prompts = None

MODE_HUB = os.environ.get("KRAKEN_HUB_URL", "http://127.0.0.1:3838")
TIMEOUT = 5


def _post(path: str, payload: dict):
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(
        MODE_HUB + path, data=body, headers={"Content-Type": "application/json"}
    )
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except Exception:
        return None


def _patch(path: str, payload: dict):
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(
        MODE_HUB + path,
        data=body,
        headers={"Content-Type": "application/json"},
        method="PATCH",
    )
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except Exception:
        return None


def state() -> dict | None:
    try:
        with urllib.request.urlopen(MODE_HUB + "/api/state", timeout=TIMEOUT) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except Exception:
        return None


def available() -> bool:
    return state() is not None


def register(name: str, model: str = "qwen2.5:7b + ornith-9b",
             capabilities: list | None = None):
    return _post("/api/agents", {
        "name": name,
        "kind": "kraken-worker",
        "model": model,
        "capabilities": capabilities or ["research", "code", "digest"],
    })


def signal(from_agent: str, body: str, to_agent: str | None = None,
           channel: str = "agent_signal"):
    payload = {"channel": channel, "from_agent": from_agent, "body": body}
    if to_agent:
        payload["to_agent"] = to_agent
    return _post("/api/messages", payload)


def _job_task_map_path(root: str) -> str:
    os.makedirs(os.path.join(root, "hub"), exist_ok=True)
    return os.path.join(root, "hub", "job_tasks.json")


def _with_job_task_lock(root: str, fn):
    import time as _t
    hub_dir = os.path.join(root, "hub")
    os.makedirs(hub_dir, exist_ok=True)
    lock = os.path.join(hub_dir, ".job_tasks.lock")
    deadline = _t.time() + 10
    while True:
        try:
            fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.close(fd)
            break
        except FileExistsError:
            try:
                if _t.time() - os.path.getmtime(lock) > 60:
                    os.remove(lock)
                    continue
            except OSError:
                pass
            if _t.time() > deadline:
                return None
            _t.sleep(0.1)
    try:
        return fn()
    finally:
        try:
            os.remove(lock)
        except OSError:
            pass


def _load_job_task_map(root: str) -> dict:
    try:
        with open(_job_task_map_path(root), "r", encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def _save_job_task_map(root: str, mapping: dict) -> None:
    try:
        with open(_job_task_map_path(root), "w", encoding="utf-8") as fh:
            json.dump(mapping, fh, indent=2, sort_keys=True)
    except Exception:
        pass


def _task_status(job_status: str) -> str:
    if job_status == "done":
        return "done"
    if job_status in {"failed", "blocked"}:
        return "blocked"
    return "in_progress"


def ensure_job_task(root: str, job: dict, worker: str | None = None) -> str | None:
    """Mirror a Kraken job into the mode hub's task board."""
    job_id = job.get("id")
    if not job_id:
        return None

    def create_or_read():
        mapping = _load_job_task_map(root)
        task_id = mapping.get(job_id)
        if task_id:
            return task_id

        input_preview = str(job.get("input", "")).replace("\n", " ")[:500]
        created = _post("/api/tasks", {
            "title": f"{job_id} {job.get('skill', 'job')}",
            "description": input_preview,
            "assigned_to": worker or "kraken-workers",
            "created_by": worker or "kraken",
        })
        if not created or not created.get("id"):
            return None
        task_id = created["id"]
        mapping[job_id] = task_id
        _save_job_task_map(root, mapping)
        return task_id

    task_id = _with_job_task_lock(root, create_or_read)
    if task_id:
        _patch(f"/api/tasks/{task_id}", {
            "status": _task_status(str(job.get("status", ""))),
            "assigned_to": worker or "kraken-workers",
        })
    return task_id


def update_job_task(root: str, job: dict, worker: str | None = None) -> None:
    task_id = ensure_job_task(root, job, worker=worker)
    if not task_id:
        return
    payload = {"status": _task_status(str(job.get("status", "")))}
    if worker:
        payload["assigned_to"] = worker
    _patch(f"/api/tasks/{task_id}", payload)


def backfill_job_tasks(root: str, jobs: list[dict], worker: str | None = None,
                       limit: int = 12) -> None:
    for job in jobs[-limit:]:
        update_job_task(root, job, worker=worker)


def claim_mission(root: str, mission_id: str) -> bool:
    """Atomically claim a mission id in the shared seen-set (lock-file guarded).
    True = this worker owns it; False = another worker got there first."""
    import time as _t
    hub_dir = os.path.join(root, "hub")
    os.makedirs(hub_dir, exist_ok=True)
    lock = os.path.join(hub_dir, ".missions.lock")
    seen_path = os.path.join(hub_dir, "missions_seen.json")
    deadline = _t.time() + 10
    while True:
        try:
            fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.close(fd)
            break
        except FileExistsError:
            try:
                if _t.time() - os.path.getmtime(lock) > 60:
                    os.remove(lock)
                    continue
            except OSError:
                pass
            if _t.time() > deadline:
                return False
            _t.sleep(0.1)
    try:
        try:
            with open(seen_path, "r", encoding="utf-8") as fh:
                seen = set(json.load(fh))
        except Exception:
            seen = set()
        if mission_id in seen:
            return False
        seen.add(mission_id)
        with open(seen_path, "w", encoding="utf-8") as fh:
            json.dump(sorted(seen), fh)
        return True
    finally:
        try:
            os.remove(lock)
        except OSError:
            pass


def _slug(text: str, default: str = "task") -> str:
    text = re.sub(r"[^A-Za-z0-9]+", "_", text.lower()).strip("_")
    return (text or default)[:48].strip("_") or default


def _game_seed_code() -> str:
    return r'''
"""Little terminal game: Star Courier.

Move on a 5x5 grid, collect three stars, avoid the storm, and return home.
Windows-friendly: no external dependencies.
"""

from __future__ import annotations

import random

WIDTH = 5
HEIGHT = 5
TARGET_SCORE = 3


def clamp(value: int, low: int, high: int) -> int:
    return max(low, min(high, value))


def move_player(pos: tuple[int, int], command: str, width: int = WIDTH, height: int = HEIGHT) -> tuple[int, int]:
    x, y = pos
    command = command.lower().strip()[:1]
    if command == "w":
        y -= 1
    elif command == "s":
        y += 1
    elif command == "a":
        x -= 1
    elif command == "d":
        x += 1
    return (clamp(x, 0, width - 1), clamp(y, 0, height - 1))


def collided(a: tuple[int, int], b: tuple[int, int]) -> bool:
    return a == b


def score_for(player: tuple[int, int], star: tuple[int, int], score: int) -> tuple[int, bool]:
    if collided(player, star):
        return score + 1, True
    return score, False


def has_won(player: tuple[int, int], score: int, home: tuple[int, int] = (0, 0)) -> bool:
    return score >= TARGET_SCORE and player == home


def random_empty(occupied: set[tuple[int, int]]) -> tuple[int, int]:
    choices = [
        (x, y)
        for y in range(HEIGHT)
        for x in range(WIDTH)
        if (x, y) not in occupied
    ]
    return random.choice(choices)


def render(player: tuple[int, int], star: tuple[int, int], storm: tuple[int, int], score: int) -> str:
    rows: list[str] = []
    for y in range(HEIGHT):
        cells: list[str] = []
        for x in range(WIDTH):
            pos = (x, y)
            if pos == player:
                cells.append("@")
            elif pos == star:
                cells.append("*")
            elif pos == storm:
                cells.append("!")
            elif pos == (0, 0):
                cells.append("H")
            else:
                cells.append(".")
        rows.append(" ".join(cells))
    return "\n".join(rows) + f"\nStars: {score}/{TARGET_SCORE}"


def main() -> None:
    print("Star Courier")
    print("Collect 3 stars, avoid !, then return to H. Move with W/A/S/D, Q quits.")
    player = (0, 0)
    score = 0
    star = random_empty({player})
    storm = random_empty({player, star})
    while True:
        print()
        print(render(player, star, storm, score))
        if has_won(player, score):
            print("You win: the stars are safely home.")
            again = input("Play again? [y/N] ").strip().lower()
            if again == "y":
                player, score = (0, 0), 0
                star = random_empty({player})
                storm = random_empty({player, star})
                continue
            return
        command = input("Move> ").strip().lower()
        if command == "q":
            print("Good flight.")
            return
        player = move_player(player, command)
        if collided(player, storm):
            print(render(player, star, storm, score))
            print("The storm caught you. Game over.")
            again = input("Try again? [y/N] ").strip().lower()
            if again == "y":
                player, score = (0, 0), 0
                star = random_empty({player})
                storm = random_empty({player, star})
                continue
            return
        score, collected = score_for(player, star, score)
        if collected:
            star = random_empty({player, storm})
            storm = random_empty({player, star})


if __name__ == "__main__":
    main()
'''.strip()


def _animated_game_seed_code() -> str:
    return r'''
"""Comet Lanterns: a tiny animated terminal dodge game."""

from __future__ import annotations

import os
import random
import time

WIDTH = 28
HEIGHT = 10
TARGET_SCORE = 8


def clamp(value: int, low: int, high: int) -> int:
    return max(low, min(high, value))


def move_player(x: int, command: str, width: int = WIDTH) -> int:
    command = command.lower().strip()[:1]
    if command == "a":
        x -= 1
    elif command == "d":
        x += 1
    return clamp(x, 0, width - 1)


def advance_comets(comets: list[tuple[int, int]]) -> list[tuple[int, int]]:
    return [(x, y + 1) for x, y in comets if y + 1 < HEIGHT]


def collided(player_x: int, comets: list[tuple[int, int]]) -> bool:
    return any(x == player_x and y == HEIGHT - 1 for x, y in comets)


def score_for(comets: list[tuple[int, int]], score: int) -> int:
    escaped = sum(1 for _, y in comets if y == HEIGHT - 1)
    return score + escaped


def has_won(score: int) -> bool:
    return score >= TARGET_SCORE


def frame(player_x: int, comets: list[tuple[int, int]], score: int, tick: int) -> str:
    rows: list[str] = []
    shimmer = "." if tick % 2 else "`"
    comet_set = set(comets)
    for y in range(HEIGHT):
        cells: list[str] = []
        for x in range(WIDTH):
            if y == HEIGHT - 1 and x == player_x:
                cells.append("^")
            elif (x, y) in comet_set:
                cells.append("*")
            else:
                cells.append(shimmer)
        rows.append("".join(cells))
    return "\n".join(rows) + f"\nLanterns saved: {score}/{TARGET_SCORE}"


def clear() -> None:
    os.system("cls" if os.name == "nt" else "clear")


def main() -> None:
    print("Comet Lanterns")
    print("A/D then Enter to move. Dodge falling * until 8 lanterns pass. Q quits.")
    input("Press Enter to begin...")
    player_x = WIDTH // 2
    comets: list[tuple[int, int]] = []
    score = 0
    tick = 0
    while True:
        if random.random() < 0.45:
            comets.append((random.randrange(WIDTH), 0))
        score = score_for(comets, score)
        comets = advance_comets(comets)
        clear()
        print(frame(player_x, comets, score, tick))
        if collided(player_x, comets):
            print("A comet cracked your lantern. Game over.")
            return
        if has_won(score):
            print("You win: the lanterns cross the sky.")
            return
        command = input("Move [A/D/Q/Enter]> ").strip().lower()
        if command == "q":
            print("Lantern watch ended.")
            return
        player_x = move_player(player_x, command)
        tick += 1
        time.sleep(0.08)


if __name__ == "__main__":
    main()
'''.strip()


def _code_payload(body: str) -> str:
    lower = body.lower()
    if "game" in lower:
        terminal_requested = any(token in lower for token in (
            "terminal", "console", "cli", "command line", "text mode", "text-mode",
        ))
        terminal_negated = re.search(
            r"\b(no|not|non|not\s+a|not\s+another)\s+[- ]?"
            r"(terminal|console|cli|command[- ]line|text[- ]mode)\b",
            lower,
        )
        terminal = terminal_requested and not terminal_negated
        if terminal:
            task = (
                body.strip()
                + "\n\nBuild a Windows-compatible terminal game using only the Python "
                  "standard library. Do not use curses. Prefer msvcrt for non-blocking "
                  "keyboard input on Windows, or a simple input-driven loop if needed. "
                  "Include pure helper functions for movement, scoring, collision, and "
                  "win/lose state so the verifier can import the module safely."
            )
            name = "animated_game" if any(token in lower for token in ("animation", "animated", "animate")) else _slug(body, "little_game")
            filename = f"{name}.py"
            module = os.path.splitext(filename)[0]
            directory = f"games/{name}"
            seed_code = _animated_game_seed_code() if name == "animated_game" else _game_seed_code()
            tests = (
                f"import pathlib\nimport {module}\n"
                f"assert hasattr({module}, 'main')\n"
                f"assert callable({module}.main)\n"
                f"source = pathlib.Path({module}.__file__).read_text(encoding='utf-8').lower()\n"
                "assert 'curses' not in source\n"
            )
        else:
            name = _slug(body, "browser_canvas_game")
            filename = "index.html"
            directory = f"games/{name}"
            seed_code = None
            extra = ""
            if game_prompts:
                try:
                    extra = "\n\n" + game_prompts.get_guidance("index.html", body)
                    if game_prompts.is_platformer_task(body):
                        extra += "\n\n" + game_prompts.CAPTAIN_COMIC_STYLE_CHECKLIST
                    scaffold = game_prompts.get_seed_scaffold_for_clone(body)
                    if scaffold:
                        extra += "\n\n" + scaffold
                except Exception:
                    pass
            task = (
                body.strip()
                + "\n\nBuild this as a polished single-file browser game in index.html. "
                  "Use HTML5 Canvas with embedded CSS and JavaScript only; no network, "
                  "CDN, build step, or external assets. Keep all code in the one HTML file. "
                  "Return raw HTML only, with no markdown code fences."
                + extra
            )
            tests = (
                "from pathlib import Path\n"
                "html = Path(__file__).with_name('index.html').read_text(encoding='utf-8').lower()\n"
                "assert '<canvas' in html\n"
                "assert 'requestanimationframe' in html\n"
                "assert 'gameloop' in html\n"
                "assert 'addeventlistener' in html\n"
                "assert any(key in html for key in ['keydown', 'pointer', 'mousemove', 'touchstart'])\n"
                "assert 'score' in html or 'hud' in html\n"
                "assert 'restart' in html or 'again' in html\n"
                "assert 'win' in html and ('lose' in html or 'game over' in html)\n"
                "assert 'audiocontext' in html or 'oscillator' in html or 'creategain' in html\n"
                "assert '<script' in html and '<style' in html\n"
                "assert 'http://' not in html and 'https://' not in html\n"
                "assert len(html) > 5000\n"
            )
            if game_prompts and game_prompts.is_platformer_task(body):
                try:
                    tests += game_prompts.platformer_test_additions()
                except Exception:
                    pass
    else:
        name = _slug(body, "forged_task")
        filename = f"{name}.py"
        module = os.path.splitext(filename)[0]
        directory = f"translated/{name}"
        task = (
            body.strip()
            + "\n\nBuild this as a Windows-compatible Python module using only the "
              "standard library. Avoid curses. Expose a callable main() when the "
              "artifact is meant to be run by an operator."
        )
        seed_code = None
        tests = (
            f"import pathlib\nimport {module}\n"
            f"assert hasattr({module}, 'main')\n"
            f"assert callable({module}.main)\n"
            f"source = pathlib.Path({module}.__file__).read_text(encoding='utf-8').lower()\n"
            "assert 'curses' not in source\n"
        )
    payload = {
        "task": task,
        "filename": filename,
        "dir": directory,
        "tests": tests,
    }
    if filename.lower().endswith((".html", ".htm")):
        payload["quality"] = "beast"
    if seed_code:
        payload["seed_code"] = seed_code
    return json.dumps(payload, ensure_ascii=False)


def _first_path_like(text: str) -> str | None:
    match = re.search(
        r"([A-Za-z0-9_.:/\\-]+\.(?:py|md|txt|json|toml|rs|js|ts|tsx|jsx|html|css)|"
        r"[A-Za-z0-9_.:/\\-]+[/\\][A-Za-z0-9_.:/\\-]+)",
        text,
    )
    return match.group(1) if match else None


def _quoted_value(text: str) -> str | None:
    match = re.search(r"['\"]([^'\"]+)['\"]", text)
    return match.group(1).strip() if match else None


def _coding_mcp_payload(body: str) -> str:
    lower = body.lower()
    path = _first_path_like(body) or "."
    if "git" in lower and "status" in lower:
        payload = {"op": "git_status", "path": path}
    elif any(token in lower for token in ("run tests", "test project", "tests", "pytest")):
        payload = {"op": "run_tests", "path": path}
    elif any(token in lower for token in ("compile", "syntax", "py_compile")):
        payload = {"op": "py_compile", "path": path}
    elif any(token in lower for token in ("search", "grep", "find text", "look for")):
        pattern = _quoted_value(body)
        if not pattern:
            for token in ("search for", "grep for", "find text", "look for", "search", "grep"):
                index = lower.find(token)
                if index >= 0:
                    pattern = body[index + len(token):].strip(" :.-")
                    break
        payload = {"op": "search", "path": path, "pattern": pattern or "TODO"}
    elif any(token in lower for token in ("read", "open file", "show file", "inspect file")) and path != ".":
        payload = {"op": "read", "path": path}
    else:
        payload = {"op": "tree", "path": path, "max_files": 120}
    return json.dumps(payload, ensure_ascii=False)


def _translate_plain_mission(body: str, known: set[str]) -> tuple[str, str] | None:
    """Best-effort natural-language intake for operator chat.

    Explicit `<skill>:` lines remain the primary protocol. This fallback turns
    ordinary requests into a single Kraken job so the room feels usable without
    making the worker a general chat bot.
    """
    body = body.strip()
    if not body:
        return None
    lower = body.lower()
    if any(token in lower for token in ("digest", "brief", "summarize folder", "summarise folder")):
        return ("prod_digest", body)
    if any(token in lower for token in ("evergreen", "distill", "vault note")):
        return ("prod_vault", body)
    if "coding_mcp" in known and any(token in lower for token in (
        "coding mcp", "mcp", "inspect", "tree", "list files", "read file",
        "open file", "show file", "search", "grep", "look for", "run tests",
        "test project", "compile", "syntax", "git status", "status of code",
        "project status", "what files",
    )):
        return ("coding_mcp", _coding_mcp_payload(body))
    if any(token in lower for token in (
        "build", "make", "write", "create", "code", "script", "tool",
        "game", "program", "app",
    )):
        if "code_forge" in known:
            return ("code_forge", _code_payload(body))
    if any(token in lower for token in ("latest", "current", "public web", "online", "internet")):
        if "research_web" in known:
            return ("research_web", body)
    if "research_local" in known:
        return ("research_local", body)
    return None


def mission_jobs(root: str | None = None) -> list[dict]:
    """Read mission messages from the mode hub. The operator types
    `<skill>: <input>` in Mission Chat; one message may contain SEVERAL
    missions (pasted batch) — every line starting with a known skill name
    opens a new mission, and following lines (e.g. multi-line code_forge
    JSON/tests) belong to the current one. Returns [{skill, input, id}]
    where id is unique per mission within a message."""
    known: set[str] = set()
    if root:
        skills_dir = os.path.join(root, "skills")
        if os.path.isdir(skills_dir):
            known = {n for n in os.listdir(skills_dir)
                     if os.path.isdir(os.path.join(skills_dir, n))}
    data = state()
    if not data:
        return []
    out = []
    for msg in data.get("messages", []):
        if msg.get("channel") != "mission" or msg.get("from_agent") != "human":
            continue
        body = (msg.get("body") or "").strip()
        missions: list[list[str]] = []  # [skill, accumulated input lines...]
        for line in body.splitlines():
            head, sep, rest = line.partition(":")
            head = head.strip()
            starts_new = sep and (
                head in known if known else (head and " " not in head))
            if starts_new:
                missions.append([head, rest.strip()])
            elif missions:
                missions[-1].append(line)
        for index, parts in enumerate(missions):
            skill, input_ = parts[0], "\n".join(parts[1:]).strip()
            if input_:
                out.append({"skill": skill, "input": input_,
                            "id": f"{msg.get('id')}#{index}"})
        if not missions:
            translated = _translate_plain_mission(body, known)
            if translated:
                skill, input_ = translated
                out.append({"skill": skill, "input": input_,
                            "id": f"{msg.get('id')}#auto"})
    return out

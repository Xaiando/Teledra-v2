#!/usr/bin/env python3
"""Kraken CLI — the operator's handle on the silent work assistant.

    python kraken.py add <skill> "<input>"    queue a job
    python kraken.py run [N]                  process up to N jobs (default 25)
    python kraken.py daemon                   silent loop (poll every 20s)
    python kraken.py worker [name]            hub-connected worker (mode hub :3838)
    python kraken.py status                   queue + last journal lines
    python kraken.py graduate-games           audit the production game inventory
    python kraken.py watch                    live watch with activity spinner
    python kraken.py skills                   list discovered skills

Run with D:\\Teledra\\.venv\\Scripts\\python.exe. See SPEC.md for the design.
"""

from __future__ import annotations

import itertools
import json
import os
import sys
import time

ROOT = os.path.abspath(os.path.dirname(__file__))
sys.path.insert(0, os.path.dirname(ROOT))  # make `kraken.kernel` importable

from kraken.kernel import llm, loop, skills
from kraken.kernel.queue import Queue


def _safe_print(*values: object, sep: str = " ", end: str = "\n") -> None:
    """Print journal/model text even when a Windows console uses cp1252."""
    text = sep.join(str(value) for value in values)
    try:
        print(text, end=end)
    except UnicodeEncodeError:
        encoding = getattr(sys.stdout, "encoding", None) or "ascii"
        safe = text.encode(encoding, errors="backslashreplace").decode(encoding)
        print(safe, end=end)


def _pid_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    if os.name == "nt":
        try:
            import ctypes
            handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)
            if not handle:
                return False
            ctypes.windll.kernel32.CloseHandle(handle)
            return True
        except Exception:
            return False
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def _acquire_pidfile(name: str) -> str | None:
    """Atomically acquire a process singleton, breaking only dead owners."""
    hub_dir = os.path.join(ROOT, "hub")
    os.makedirs(hub_dir, exist_ok=True)
    path = os.path.join(hub_dir, f"{name}.pid")
    for _ in range(2):
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.write(fd, str(os.getpid()).encode("ascii"))
            os.close(fd)
            return path
        except FileExistsError:
            try:
                owner = int(open(path, "r", encoding="ascii").read().strip())
            except (OSError, ValueError):
                owner = 0
            if _pid_alive(owner):
                return None
            try:
                os.remove(path)
            except OSError:
                return None
    return None


def _release_pidfile(path: str | None) -> None:
    if not path:
        return
    try:
        owner = int(open(path, "r", encoding="ascii").read().strip())
    except (OSError, ValueError):
        owner = 0
    if owner == os.getpid():
        try:
            os.remove(path)
        except OSError:
            pass


def _journal_jsonl_path() -> str | None:
    journal_dir = os.path.join(ROOT, "journal")
    if not os.path.isdir(journal_dir):
        return None
    files = sorted(
        name for name in os.listdir(journal_dir)
        if len(name) == 14 and name[:8].isdigit() and name.endswith(".jsonl")
    )
    return os.path.join(journal_dir, files[-1]) if files else None


def _supervisor_active() -> bool:
    path = os.path.join(ROOT, "jobs", ".supervisor.lock")
    try:
        owner = int(open(path, "r", encoding="ascii").read().split()[0])
    except (OSError, ValueError, IndexError):
        return False
    return _pid_alive(owner)


def _read_payload_arg(arg: str) -> str:
    """Read operator payloads from files without PowerShell quote hazards."""
    if arg.startswith("@"):
        path = arg[1:]
    else:
        path = arg
    with open(path, "r", encoding="utf-8-sig") as fh:
        return fh.read().strip()


def cmd_add(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: kraken.py add <skill> \"<input>\" | --input-file <path> | @<path>")
        return 2
    skill = argv[0]
    if len(argv) >= 3 and argv[1] in {"--input-file", "-f"}:
        payload = _read_payload_arg(argv[2])
    elif len(argv) == 2 and argv[1].startswith("@"):
        payload = _read_payload_arg(argv[1])
    else:
        payload = " ".join(argv[1:])
    job = Queue(ROOT).add(skill, payload)
    print(f"queued {job['id']} -> {job['skill']}")
    return 0


def _drain(max_jobs: int, quiet: bool) -> int:
    # supervised when available: per-job timeout kill, workdir/vault revert
    try:
        from kraken.kernel import supervisor
        return supervisor.drain(ROOT, max_jobs=max_jobs, quiet=quiet)
    except ImportError:
        return loop.drain(ROOT, max_jobs=max_jobs, quiet=quiet)


def cmd_run(argv: list[str]) -> int:
    if not llm.available():
        print("ollama is not reachable at localhost:11434 - start it first")
        return 1
    llm.ensure_models()  # guarantee qwen2.5:7b + Ornith (not pure ollama fallback)
    limit = int(argv[0]) if argv else 25
    count = _drain(limit, quiet=False)
    print(f"processed {count} job(s)")
    return 0


def cmd_daemon(argv: list[str]) -> int:
    pidfile = _acquire_pidfile("kraken-daemon")
    if pidfile is None:
        print("kraken daemon is already running; refusing to create a competing worker")
        return 1
    print("kraken daemon: silent watch, ctrl-c to stop")
    queue = Queue(ROOT)
    try:
        while True:
            # The ordinary daemon used to omit stale-claim recovery entirely,
            # leaving dead workers permanently marked running.
            queue.reap_stale()
            if llm.available():
                llm.ensure_models()  # guarantee qwen2.5:7b + Ornith
                _drain(10, quiet=True)
            time.sleep(20)
    except KeyboardInterrupt:
        return 0
    finally:
        _release_pidfile(pidfile)


def cmd_status(argv: list[str]) -> int:
    jobs = Queue(ROOT).all()
    by_status: dict[str, int] = {}
    for job in jobs:
        by_status[job["status"]] = by_status.get(job["status"], 0) + 1
    _safe_print("queue:", by_status or "empty")
    stale_running = []
    now = time.time()
    for job in jobs:
        if job.get("status") != "running":
            continue
        try:
            updated = time.mktime(time.strptime(job.get("updated", ""), "%Y-%m-%dT%H:%M:%S"))
        except (TypeError, ValueError):
            updated = 0
        if now - updated > 900:
            stale_running.append(job["id"])
    _safe_print(f"supervisor: {'ACTIVE' if _supervisor_active() else 'IDLE'}; stale running: {len(stale_running)}")
    for job in jobs[-8:]:
        line = f"  {job['id']} {job['status']:<8} {job['skill']:<16} {job['input'][:60]}"
        if job.get("output"):
            line += f" -> {job['output']}"
        _safe_print(line)
    path = _journal_jsonl_path()
    if path:
        with open(path, "r", encoding="utf-8") as fh:
            tail = fh.readlines()[-5:]
        _safe_print(f"journal ({os.path.basename(path)}):")
        for line in tail:
            entry = json.loads(line)
            _safe_print("  ", entry.get("ts"), entry.get("job"),
                        entry.get("verdict", entry.get("note", "")))
    return 0


def _clear_screen() -> None:
    os.system('cls' if os.name == 'nt' else 'clear')


def cmd_watch(argv: list[str]) -> int:
    """Live watch mode with activity indicator (spinner when agents are busy)."""
    print("Kraken Watch — live swarm monitor")
    print("Press Ctrl+C to exit. Spinner rotates when jobs are running or recent activity.\n")
    spinner = itertools.cycle(['|', '/', '-', '\\'])
    poll_interval = 1.5
    last_journal_pos = 0
    last_activity_seconds = None
    last_activity_str = ""
    recent_journal_lines = []

    try:
        while True:
            _clear_screen()
            print("KRAKEN LIVE  " + next(spinner) + "   (Ctrl+C to stop)")

            # Queue summary
            jobs = Queue(ROOT).all()
            by_status: dict[str, int] = {}
            running = 0
            for job in jobs:
                s = job.get("status", "unknown")
                by_status[s] = by_status.get(s, 0) + 1
                if s == "running":
                    running += 1

            # Load journal info early for activity detection
            last_activity_str = ""
            recent_journal_lines = []
            jpath = _journal_jsonl_path()
            if jpath:
                try:
                    with open(jpath, "r", encoding="utf-8") as fh:
                        all_lines = fh.readlines()
                        if all_lines:
                            last = json.loads(all_lines[-1])
                            last_ts = last.get("ts")
                            if last_ts:
                                from datetime import datetime
                                try:
                                    last_dt = datetime.fromisoformat(last_ts)
                                    delta = (datetime.now() - last_dt).total_seconds()
                                    last_activity_seconds = delta
                                    last_activity_str = f"   |   last journal: {int(delta)}s ago"
                                except (TypeError, ValueError):
                                    pass
                            recent_journal_lines = all_lines[-8:]
                except (OSError, json.JSONDecodeError):
                    pass

            # A stale queue label is not activity. Require a live supervisor
            # lease or a genuinely recent journal heartbeat.
            recent_activity = last_activity_seconds is not None and last_activity_seconds < 45
            busy = _supervisor_active() or recent_activity
            status_line = "BUSY" if busy else "IDLE"
            print(f"Status: {status_line}   |   queue: {by_status or 'empty'}   |   recorded running: {running}")

            print(f"Recent activity{last_activity_str}")

            # Recent jobs
            print("\nRecent jobs:")
            for job in jobs[-6:]:
                line = f"  {job['id']} {job['status']:<8} {job['skill']:<16} {str(job.get('input',''))[:55]}"
                if job.get("output"):
                    line += f" -> {job['output']}"
                print(line)

            # Journal tail (use the recent lines we already loaded for activity)
            if recent_journal_lines:
                print(f"\nJournal tail:")
                for line in recent_journal_lines:
                    try:
                        entry = json.loads(line)
                        ts = entry.get("ts", "")[-8:]
                        jid = entry.get("job", "")
                        note = entry.get("verdict") or entry.get("note", "") or entry.get("reason", "")
                        print(f"  {ts} {jid} {note[:70]}")
                    except Exception:
                        pass
            else:
                # fallback incremental if first run
                path = _journal_jsonl_path()
                if path:
                    try:
                        with open(path, "r", encoding="utf-8") as fh:
                            fh.seek(last_journal_pos)
                            new_lines = fh.readlines()
                            last_journal_pos = fh.tell()
                        if new_lines:
                            print(f"\nJournal tail:")
                            for line in new_lines[-8:]:
                                try:
                                    entry = json.loads(line)
                                    ts = entry.get("ts", "")[-8:]
                                    jid = entry.get("job", "")
                                    note = entry.get("verdict") or entry.get("note", "") or entry.get("reason", "")
                                    print(f"  {ts} {jid} {note[:70]}")
                                except (json.JSONDecodeError, TypeError):
                                    pass
                    except OSError:
                        pass

            # Hint
            if busy:
                print("\n[ Agents are working — watch for 'asking Ornith' or 'repair' lines ]")
            else:
                print("\n[ Idle — queue new jobs with 'add' or send missions via hub ]")
            print("\nTip: A long generation is active only while the supervisor lease and journal heartbeat are current.")

            time.sleep(poll_interval)
    except KeyboardInterrupt:
        print("\nWatch stopped.")
        return 0


def cmd_worker(argv: list[str]) -> int:
    """Activated taskforce worker: registers on the mode hub (port 3838),
    pulls `skill: input` missions from its Mission Chat into the queue,
    drains supervised, and signals every job outcome back to the hub."""
    from kraken.kernel import hub
    from kraken.kernel import supervisor

    name = argv[0] if argv else "kraken-worker-1"
    # second arg = work folder (free-rein zone); propagates to skills via env
    if len(argv) > 1 and argv[1].strip():
        os.environ["KRAKEN_WORKSPACE"] = os.path.abspath(argv[1].strip())
    workspace = os.environ.get("KRAKEN_WORKSPACE", os.path.join(ROOT, "workspace"))
    os.makedirs(workspace, exist_ok=True)

    # singleton per worker name: duplicate launches (bat + background + manual)
    # cause ghost claims with stale code. Refuse to double-start.
    os.makedirs(os.path.join(ROOT, "hub"), exist_ok=True)
    pidfile = os.path.join(ROOT, "hub", f"{name}.pid")
    try:
        old_pid = int(open(pidfile, "r", encoding="utf-8").read().strip())
        # NOTE: os.kill(pid, 0) TERMINATES on Windows — use OpenProcess instead
        import ctypes
        handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, old_pid)
        if handle:
            ctypes.windll.kernel32.CloseHandle(handle)
            print(f"{name} is already running (pid {old_pid}); refusing to duplicate.")
            return 1
    except (OSError, ValueError):
        pass
    with open(pidfile, "w", encoding="utf-8") as fh:
        fh.write(str(os.getpid()))

    hub.register(name)
    hub.signal(name, f"{name} activated. workspace={workspace}. "
                     f"Post '<skill>: <input>' or plain requests; I will translate and queue them.")
    print(f"{name} on duty (hub {hub.MODE_HUB}, workspace {workspace}); ctrl-c to stop")

    queue = Queue(ROOT)
    reaped = queue.reap_stale()
    if reaped:
        hub.signal(name, f"reaped {reaped} stale running job(s) on startup")
    hub.backfill_job_tasks(ROOT, queue.all(), worker=name)
    while True:
        try:
            queue.reap_stale()  # catch mid-session worker deaths too
            # re-register each cycle: an API write forces the hub to reload
            # state from disk, so missions typed in the desktop (which writes
            # the file directly) become visible to polling. Silent - no chat.
            hub.register(name)
            active = {(j["skill"], j["input"]) for j in queue.all()
                      if j["status"] in ("queued", "running")}
            for mission in hub.mission_jobs(ROOT):
                # shared, lock-guarded claim — exactly one worker queues it
                if not hub.claim_mission(ROOT, mission["id"]):
                    continue
                # idempotence: identical work already in flight -> skip
                if (mission["skill"], mission["input"]) in active:
                    hub.signal(name, f"skipped duplicate mission (already in flight): "
                                     f"{mission['skill']}: {mission['input'][:80]}")
                    continue
                job = queue.add(mission["skill"], mission["input"])
                hub.update_job_task(ROOT, job, worker=name)
                hub.signal(name, f"queued {job['id']} -> {mission['skill']}: "
                                 f"{mission['input'][:120]}")

            if llm.available():
                while True:
                    seen_progress: set[str] = set()

                    def progress(job: dict, text: str) -> None:
                        key = f"{job.get('id')}|{text}"
                        if key in seen_progress:
                            return
                        seen_progress.add(key)
                        hub.signal(name, f"{job.get('id')} {job.get('skill')} working: {text}")

                    job = supervisor.run_once(ROOT, progress=progress)
                    if job is None:
                        break
                    hub.update_job_task(ROOT, job, worker=name)
                    note = f"{job['id']} {job['skill']} -> {job['status']}"
                    if job.get("output"):
                        note += f" ({job['output']})"
                        if job.get("skill") == "code_forge" and str(job.get("output", "")).endswith(".py"):
                            note += (
                                f"\nRun: D:\\Teledra\\.venv\\Scripts\\python.exe "
                                f"{job['output']}"
                            )
                        elif job.get("skill") == "code_forge" and str(job.get("output", "")).lower().endswith((".html", ".htm")):
                            note += f"\nOpen: {os.path.abspath(job['output'])}"
                    hub.signal(name, note)
            time.sleep(15)
        except KeyboardInterrupt:
            hub.signal(name, f"{name} standing down.")
            return 0


def cmd_skills(argv: list[str]) -> int:
    for name, skill in skills.discover(ROOT).items():
        print(f"  {name:<18} harness={skill.harness or '-':<18} timeout={skill.timeout_s}s")
    return 0


def cmd_graduate_games(argv: list[str]) -> int:
    """Write a truthful per-game acceptance manifest without mutating games."""
    from pathlib import Path
    from kraken.harness import game_graduation

    run_browser = "--no-browser" not in argv
    output = Path(ROOT) / "output" / "game_acceptance_manifest.json"
    if "--output" in argv:
        index = argv.index("--output")
        if index + 1 >= len(argv):
            print("usage: kraken.py graduate-games [--no-browser] [--output <path>]")
            return 2
        output = Path(argv[index + 1])
        if not output.is_absolute():
            output = Path(ROOT) / output

    manifest = game_graduation.build_manifest(Path(ROOT), run_browser=run_browser)
    game_graduation.write_manifest(manifest, output)
    summary = manifest["summary"]
    print(
        f"game graduation: {summary['graduated']}/{summary['browser_games']} browser games accepted; "
        f"manifest={output.resolve()}"
    )
    return 0 if summary["all_browser_games_graduated"] else 1


def main() -> int:
    commands = {"add": cmd_add, "run": cmd_run, "daemon": cmd_daemon,
                "worker": cmd_worker, "status": cmd_status, "watch": cmd_watch,
                "skills": cmd_skills, "graduate-games": cmd_graduate_games}
    if len(sys.argv) < 2 or sys.argv[1] not in commands:
        print(__doc__)
        return 2
    return commands[sys.argv[1]](sys.argv[2:])


if __name__ == "__main__":
    raise SystemExit(main())

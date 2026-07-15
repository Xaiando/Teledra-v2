"""Supervised Kraken runner.

Wraps the single-pass worker loop with the Wizard-pattern outer guard:
per-job timeout kill, progress-or-reject, and best-effort workdir revert.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROJECT_ROOT = ROOT.parent

# Clean sys.path to prevent script directory (kernel/) from colliding with stdlib (like queue)
script_dir = str(Path(__file__).resolve().parent)
sys.path = [p for p in sys.path if p and str(Path(p).resolve()) != script_dir]

if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from kraken.kernel import loop, skills as skills_mod
from kraken.kernel.queue import MAX_ATTEMPTS, Queue

SUPERVISOR_LOCK = ".supervisor.lock"


def _now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S")


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


def _acquire_supervisor_lease(root: str) -> str | None:
    """Allow exactly one model-backed Kraken job across daemon/manual runners."""
    path = os.path.join(root, "jobs", SUPERVISOR_LOCK)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    for _ in range(2):
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.write(fd, f"{os.getpid()} {_now()}".encode("ascii"))
            os.close(fd)
            return path
        except FileExistsError:
            try:
                owner = int(open(path, "r", encoding="ascii").read().split()[0])
            except (OSError, ValueError, IndexError):
                owner = 0
            if _pid_alive(owner):
                return None
            try:
                os.remove(path)
            except OSError:
                return None
    return None


def _release_supervisor_lease(path: str | None) -> None:
    if not path:
        return
    try:
        owner = int(open(path, "r", encoding="ascii").read().split()[0])
    except (OSError, ValueError, IndexError):
        owner = 0
    if owner == os.getpid():
        try:
            os.remove(path)
        except OSError:
            pass


def _journal(root: str, entry: dict) -> None:
    path = os.path.join(root, "journal", time.strftime("%Y%m%d") + ".jsonl")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    payload = dict(entry)
    payload["ts"] = _now()
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(payload, ensure_ascii=False) + "\n")


def _journal_path(root: str) -> str:
    return os.path.join(root, "journal", time.strftime("%Y%m%d") + ".jsonl")


def _tail_job_journal(root: str, job_id: str, offset: int) -> tuple[int, list[dict]]:
    path = _journal_path(root)
    if not os.path.exists(path):
        return offset, []
    try:
        size = os.path.getsize(path)
        if offset > size:
            offset = 0
        entries: list[dict] = []
        with open(path, "r", encoding="utf-8") as fh:
            fh.seek(offset)
            for line in fh:
                try:
                    entry = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if entry.get("job") == job_id:
                    entries.append(entry)
            return fh.tell(), entries
    except OSError:
        return offset, []


def _progress_text(entry: dict) -> str | None:
    if entry.get("note"):
        return str(entry["note"])
    if entry.get("verdict"):
        text = f"verdict: {entry.get('verdict')}"
        if entry.get("reasons"):
            text += " - " + "; ".join(str(reason).splitlines()[0] for reason in entry["reasons"][:2])
        elif entry.get("notes"):
            text += " - " + str(entry.get("notes")).splitlines()[0]
        return text
    if entry.get("reason"):
        return str(entry["reason"])
    return None


def _latest_job(root: str, job_id: str) -> dict | None:
    for job in Queue(root).all():
        if job.get("id") == job_id:
            return job
    return None


def _next_job(root: str) -> dict | None:
    queued = [job for job in Queue(root).all() if job.get("status") == "queued"]
    if not queued:
        return None
    return sorted(queued, key=lambda job: job.get("created", ""))[0]


def _timeout_for(root: str, skill_name: str, default: int) -> int:
    skill = skills_mod.discover(root).get(skill_name)
    timeout = int(getattr(skill, "timeout_s", default) or default)
    md_path = os.path.join(root, "skills", skill_name, "SKILL.md")
    try:
        with open(md_path, "r", encoding="utf-8-sig") as fh:
            match = re.match(r"\s*---\s*\n(.*?)\n---", fh.read(), re.DOTALL)
        if match:
            for line in match.group(1).splitlines():
                key, _, value = line.partition(":")
                if key.strip() == "timeout_s":
                    timeout = int(value.strip())
                    break
    except (OSError, ValueError):
        pass
    return timeout


def _copytree(src: str, dst: str) -> None:
    if os.path.isdir(src):
        shutil.copytree(src, dst)


def _snapshot_workdir(root: str, job: dict) -> str | None:
    workdir = Queue(root).workdir(job)
    if not os.path.exists(workdir):
        return None
    tmp = tempfile.mkdtemp(prefix=f"kraken-{job['id']}-")
    snap = os.path.join(tmp, "workdir")
    _copytree(workdir, snap)
    return snap


def _restore_workdir(root: str, job: dict, snapshot: str | None) -> None:
    if snapshot is None:
        return
    workdir = Queue(root).workdir(job)
    if os.path.exists(workdir):
        shutil.rmtree(workdir, ignore_errors=True)
    shutil.copytree(snapshot, workdir)


def _vault_snapshot(root: str) -> set[str]:
    vault = os.path.join(root, "vault")
    if not os.path.isdir(vault):
        return set()
    return {os.path.join(vault, name) for name in os.listdir(vault)}


def _revert_new_vault_files(root: str, before: set[str]) -> None:
    vault = os.path.join(root, "vault")
    if not os.path.isdir(vault):
        return
    for name in os.listdir(vault):
        path = os.path.join(vault, name)
        if path not in before and os.path.isfile(path):
            try:
                os.remove(path)
            except OSError:
                pass


def _reject_job(root: str, job: dict, reason: str, snapshot: str | None,
                vault_before: set[str]) -> dict:
    _restore_workdir(root, job, snapshot)
    _revert_new_vault_files(root, vault_before)
    latest = _latest_job(root, job["id"]) or job
    latest["attempts"] = int(latest.get("attempts", 0)) + 1
    latest.setdefault("feedback", []).append(reason)
    latest["status"] = "queued" if latest["attempts"] < MAX_ATTEMPTS else "failed"
    Queue(root).update(latest)
    _journal(root, {
        "job": latest["id"],
        "skill": latest.get("skill"),
        "verdict": latest["status"],
        "supervisor": True,
        "reason": reason,
    })
    return latest


def _made_progress(before: dict, after: dict | None) -> bool:
    if after is None:
        return False
    if after.get("status") in {"done", "failed", "blocked", "queued"}:
        if after.get("status") != "running":
            return True
    watched = ("attempts", "feedback", "output", "updated")
    return any(after.get(key) != before.get(key) for key in watched)


def _worker_once(root: str, quiet: bool) -> int:
    result = loop.run_once(root, quiet=quiet)
    return 0 if result is not None else 3


def _run_once_with_lease(root: str, *, default_timeout: int = 300, quiet: bool = True,
                         progress=None) -> dict | None:
    job = _next_job(root)
    if job is None:
        return None

    timeout_s = _timeout_for(root, job["skill"], default_timeout)
    snapshot = _snapshot_workdir(root, job)
    vault_before = _vault_snapshot(root)
    started = time.time()
    journal_offset = os.path.getsize(_journal_path(root)) if os.path.exists(_journal_path(root)) else 0
    last_heartbeat = started
    if progress:
        progress(job, f"started {job['skill']} (timeout {timeout_s}s)")

    cmd = [
        sys.executable,
        os.path.abspath(__file__),
        "--worker-once",
        "--root",
        os.path.abspath(root),
        "--quiet" if quiet else "--no-quiet",
    ]
    proc = subprocess.Popen(
        cmd,
        cwd=os.path.abspath(root),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    timed_out = False
    while proc.poll() is None:
        elapsed = time.time() - started
        if elapsed >= timeout_s:
            timed_out = True
            break
        time.sleep(1.0)
        if progress:
            journal_offset, entries = _tail_job_journal(root, job["id"], journal_offset)
            for entry in entries[-6:]:
                text = _progress_text(entry)
                if text:
                    progress(job, text[:700])
            if time.time() - last_heartbeat >= 30:
                latest = _latest_job(root, job["id"]) or job
                progress(job, f"still working ({int(elapsed)}s elapsed, status {latest.get('status')})")
                last_heartbeat = time.time()

    if timed_out:
        proc.kill()
        stdout, stderr = proc.communicate()
        reason = f"supervisor timeout after {timeout_s}s"
        if stderr.strip():
            reason += f"; stderr: {stderr.strip()[-500:]}"
        if progress:
            progress(job, reason)
        return _reject_job(root, job, reason, snapshot, vault_before)
    stdout, stderr = proc.communicate()

    elapsed_ms = int((time.time() - started) * 1000)
    if progress:
        journal_offset, entries = _tail_job_journal(root, job["id"], journal_offset)
        for entry in entries[-8:]:
            text = _progress_text(entry)
            if text:
                progress(job, text[:700])
    after = _latest_job(root, job["id"])
    if proc.returncode not in (0, 3) and not _made_progress(job, after):
        reason = f"worker exited {proc.returncode} without progress"
        if stderr.strip():
            reason += f"; stderr: {stderr.strip()[-500:]}"
        return _reject_job(root, job, reason, snapshot, vault_before)
    if not _made_progress(job, after):
        return _reject_job(root, job, "worker exited without state progress", snapshot, vault_before)

    _journal(root, {
        "job": job["id"],
        "skill": job["skill"],
        "ms": elapsed_ms,
        "supervisor": True,
        "verdict": (after or {}).get("status", "unknown"),
        "stdout": stdout.strip()[-400:],
    })
    return after


def run_once(root: str, *, default_timeout: int = 300, quiet: bool = True,
             progress=None) -> dict | None:
    lease = _acquire_supervisor_lease(root)
    if lease is None:
        return None
    try:
        return _run_once_with_lease(
            root,
            default_timeout=default_timeout,
            quiet=quiet,
            progress=progress,
        )
    finally:
        _release_supervisor_lease(lease)


def drain(root: str, *, max_jobs: int = 25, default_timeout: int = 300,
          quiet: bool = True) -> int:
    count = 0
    while count < max_jobs:
        if run_once(root, default_timeout=default_timeout, quiet=quiet) is None:
            break
        count += 1
    return count


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", nargs="?", default="run",
                        choices=["once", "run", "daemon"])
    parser.add_argument("max_jobs", nargs="?", type=int, default=25)
    parser.add_argument("--root", default=str(ROOT))
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--poll", type=int, default=20)
    parser.add_argument("--worker-once", action="store_true")
    parser.add_argument("--quiet", action="store_true", default=True)
    parser.add_argument("--no-quiet", dest="quiet", action="store_false")
    args = parser.parse_args(argv)

    if args.worker_once:
        return _worker_once(args.root, args.quiet)

    if args.command == "once":
        return 0 if run_once(args.root, default_timeout=args.timeout, quiet=args.quiet) else 3
    if args.command == "run":
        print(f"supervised processed {drain(args.root, max_jobs=args.max_jobs, default_timeout=args.timeout, quiet=args.quiet)} job(s)")
        return 0

    print("kraken supervisor daemon: ctrl-c to stop")
    while True:
        try:
            drain(args.root, max_jobs=args.max_jobs, default_timeout=args.timeout, quiet=True)
            time.sleep(args.poll)
        except KeyboardInterrupt:
            return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Kraken worker loop — claim, execute, verify, journal, recurse. Repeat.

This is the single-pass engine. supervisor.py (Codex's lane) wraps it with
per-job wall-clock timeouts, auto-revert, and progress-or-reject for daemon
duty; run_once() stays importable and testable on its own.
"""

from __future__ import annotations

import json
import os
import sys
import time
import traceback

from . import harness, llm, skills as skills_mod
from .queue import MAX_ATTEMPTS, MAX_CHILDREN, MAX_DEPTH, Queue


def _journal(root: str, entry: dict) -> None:
    path = os.path.join(root, "journal", time.strftime("%Y%m%d") + ".jsonl")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    entry["ts"] = time.strftime("%Y-%m-%dT%H:%M:%S")
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(entry, ensure_ascii=False) + "\n")


def run_once(root: str, quiet: bool = True) -> dict | None:
    """Process one job. Returns the finished job dict, or None if queue empty."""
    queue = Queue(root)
    job = queue.claim()
    if job is None:
        return None

    def log(msg: str) -> None:
        _journal(root, {"job": job["id"], "note": msg})
        if not quiet:
            line = f"[{job['id']}] {msg}"
            try:
                print(line)
            except UnicodeEncodeError:
                encoding = getattr(sys.stdout, "encoding", None) or "ascii"
                safe = line.encode(encoding, errors="backslashreplace").decode(encoding)
                print(safe)

    registry = skills_mod.discover(root)
    skill = registry.get(job["skill"])
    started = time.time()

    if skill is None:
        job["status"] = "blocked"
        job["feedback"].append(f"unknown skill: {job['skill']}")
        queue.update(job)
        _journal(root, {"job": job["id"], "skill": job["skill"],
                        "verdict": "blocked", "reason": "unknown skill"})
        return job

    workspace = os.environ.get(
        "KRAKEN_WORKSPACE", os.path.join(root, "workspace"))
    os.makedirs(workspace, exist_ok=True)
    ctx = {
        "llm": llm,
        "root": root,
        "workdir": queue.workdir(job),
        "workspace": workspace,  # shared project dir for coordinated work
        "log": log,
    }

    try:
        module = skill.load()
        result = module.execute(job, ctx) or {}
    except Exception:
        result = {"ok": False, "notes": "skill crashed:\n" + traceback.format_exc(limit=4)}

    # a skill may defer: "not my turn yet" (e.g. a join waiting on siblings).
    # Deferred jobs go back to queued WITHOUT burning an attempt.
    if result.get("defer"):
        job["status"] = "queued"
        job["defer_count"] = job.get("defer_count", 0) + 1
        if job["defer_count"] > 120:
            job["status"] = "failed"
            job["feedback"].append("deferred too long; giving up")
        _journal(root, {"job": job["id"], "skill": job["skill"],
                        "verdict": "deferred", "notes": result.get("notes", "")[:200]})
        queue.update(job)
        return job

    verdict = harness.verify(job, result, ctx, skill.harness)
    elapsed_ms = int((time.time() - started) * 1000)

    if verdict["passed"]:
        job["status"] = "done"
        job["output"] = result.get("output")
        # recursion, bounded
        children = (result.get("children") or [])[: min(MAX_CHILDREN, skill.max_children)]
        spawned = []
        if job["depth"] < MAX_DEPTH:
            for child in children:
                if child.get("skill") and child.get("input"):
                    spawned.append(
                        queue.add(child["skill"], child["input"],
                                  parent=job["id"], depth=job["depth"] + 1)["id"]
                    )
        elif children:
            log(f"depth cap hit; dropped {len(children)} children")
        _journal(root, {"job": job["id"], "skill": job["skill"], "ms": elapsed_ms,
                        "verdict": "done", "output": result.get("output"),
                        "children": spawned, "notes": result.get("notes", "")[:400]})
    else:
        job["attempts"] += 1
        job["feedback"].extend(verdict["reasons"])
        job["status"] = "queued" if job["attempts"] < MAX_ATTEMPTS else "failed"
        _journal(root, {"job": job["id"], "skill": job["skill"], "ms": elapsed_ms,
                        "verdict": job["status"], "reasons": verdict["reasons"],
                        "notes": result.get("notes", "")[:400]})

    queue.update(job)
    return job


def drain(root: str, max_jobs: int = 25, quiet: bool = True) -> int:
    """Run until the queue is empty or max_jobs processed. Returns count."""
    done = 0
    while done < max_jobs:
        if run_once(root, quiet=quiet) is None:
            break
        done += 1
    return done

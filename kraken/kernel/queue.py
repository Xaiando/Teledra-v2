"""Kraken job queue — file-backed, single-writer, crash-tolerant.

jobs/jobs.jsonl holds one JSON object per job (last line for an id wins, so
updates are appends; compact() rewrites). A lock file makes concurrent CLI +
daemon safe enough for a one-box kingdom.
"""

from __future__ import annotations

import json
import os
import time
import uuid

MAX_DEPTH = 3
MAX_CHILDREN = 5
MAX_ATTEMPTS = 2

STATUSES = ("queued", "running", "done", "failed", "blocked")


class Queue:
    def __init__(self, root: str):
        self.dir = os.path.join(root, "jobs")
        os.makedirs(self.dir, exist_ok=True)
        self.path = os.path.join(self.dir, "jobs.jsonl")
        self.lock_path = os.path.join(self.dir, ".lock")

    # -- locking (best-effort, PID-stamped) ---------------------------------
    def _lock(self, timeout: float = 10.0) -> None:
        deadline = time.time() + timeout
        while True:
            try:
                fd = os.open(self.lock_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                os.write(fd, str(os.getpid()).encode())
                os.close(fd)
                return
            except FileExistsError:
                # stale lock (> 5 min) gets broken — a crashed run must not wedge us
                try:
                    if time.time() - os.path.getmtime(self.lock_path) > 300:
                        os.remove(self.lock_path)
                        continue
                except OSError:
                    pass
                if time.time() > deadline:
                    raise TimeoutError("queue lock busy")
                time.sleep(0.2)

    def _unlock(self) -> None:
        try:
            os.remove(self.lock_path)
        except OSError:
            pass

    # -- core ----------------------------------------------------------------
    def _read_all(self) -> dict:
        jobs: dict[str, dict] = {}
        if not os.path.exists(self.path):
            return jobs
        with open(self.path, "r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    job = json.loads(line)
                    jobs[job["id"]] = job
                except (json.JSONDecodeError, KeyError):
                    continue  # torn write; append-only means older state survives
        return jobs

    def _append(self, job: dict) -> None:
        job["updated"] = _now()
        with open(self.path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(job, ensure_ascii=False) + "\n")

    # -- API -----------------------------------------------------------------
    def add(self, skill: str, input_: str, parent: str | None = None,
            depth: int = 0) -> dict:
        job = {
            "id": f"k-{time.strftime('%Y%m%d')}-{uuid.uuid4().hex[:6]}",
            "skill": skill,
            "input": input_,
            "status": "queued",
            "parent": parent,
            "depth": depth,
            "attempts": 0,
            "feedback": [],
            "created": _now(),
            "updated": _now(),
        }
        self._lock()
        try:
            self._append(job)
        finally:
            self._unlock()
        return job

    def claim(self) -> dict | None:
        """Atomically move the oldest queued job to running and return it."""
        self._lock()
        try:
            jobs = self._read_all()
            queued = [j for j in jobs.values() if j["status"] == "queued"]
            if not queued:
                return None
            job = sorted(queued, key=lambda j: j["created"])[0]
            job["status"] = "running"
            self._append(job)
            return job
        finally:
            self._unlock()

    def update(self, job: dict) -> None:
        self._lock()
        try:
            self._append(job)
        finally:
            self._unlock()

    def all(self) -> list[dict]:
        return sorted(self._read_all().values(), key=lambda j: j["created"])

    def workdir(self, job: dict) -> str:
        path = os.path.join(self.dir, job["id"])
        os.makedirs(path, exist_ok=True)
        return path

    def reap_stale(self, max_running_secs: int = 900) -> int:
        """Reset jobs stuck in 'running' (worker killed mid-job) back to queued
        so they retry. Threshold sits well above any single skill timeout, so a
        legitimately-running job is never reaped. Returns count reaped."""
        cutoff = time.time() - max_running_secs
        reaped = 0
        self._lock()
        try:
            for job in self._read_all().values():
                if job.get("status") != "running":
                    continue
                try:
                    updated = time.mktime(time.strptime(
                        job.get("updated", ""), "%Y-%m-%dT%H:%M:%S"))
                except (ValueError, TypeError):
                    updated = 0
                if updated < cutoff:
                    job["status"] = "queued"
                    job.setdefault("feedback", []).append(
                        "reaped: stuck in running (worker likely killed mid-job)")
                    self._append(job)
                    reaped += 1
        finally:
            self._unlock()
        return reaped

    def compact(self) -> None:
        self._lock()
        try:
            jobs = self._read_all()
            tmp = self.path + ".tmp"
            with open(tmp, "w", encoding="utf-8") as fh:
                for job in sorted(jobs.values(), key=lambda j: j["created"]):
                    fh.write(json.dumps(job, ensure_ascii=False) + "\n")
            os.replace(tmp, self.path)
        finally:
            self._unlock()


def _now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S")

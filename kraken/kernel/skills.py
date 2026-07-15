"""Kraken skill registry — a skill is a folder with SKILL.md + run.py."""

from __future__ import annotations

import importlib.util
import os
import re


class Skill:
    def __init__(self, name: str, path: str, meta: dict):
        self.name = name
        self.path = path
        self.meta = meta
        self.timeout_s = int(meta.get("timeout_s", 300))
        self.max_children = int(meta.get("max_children", 5))
        self.harness = meta.get("harness", "")

    def load(self):
        run_path = os.path.join(self.path, "run.py")
        spec = importlib.util.spec_from_file_location(f"kraken_skill_{self.name}", run_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        if not hasattr(module, "execute"):
            raise RuntimeError(f"skill {self.name}: run.py lacks execute(job, ctx)")
        return module


def _parse_frontmatter(md_path: str) -> dict:
    """Minimal `key: value` frontmatter between --- fences in SKILL.md."""
    meta: dict = {}
    try:
        with open(md_path, "r", encoding="utf-8") as fh:
            text = fh.read()
    except OSError:
        return meta
    match = re.match(r"\s*---\s*\n(.*?)\n---", text, re.DOTALL)
    if not match:
        return meta
    for line in match.group(1).splitlines():
        if ":" in line:
            key, _, value = line.partition(":")
            meta[key.strip()] = value.strip()
    return meta


def discover(root: str) -> dict[str, Skill]:
    skills_dir = os.path.join(root, "skills")
    found: dict[str, Skill] = {}
    if not os.path.isdir(skills_dir):
        return found
    for name in sorted(os.listdir(skills_dir)):
        path = os.path.join(skills_dir, name)
        if not os.path.isdir(path) or not os.path.exists(os.path.join(path, "run.py")):
            continue
        meta = _parse_frontmatter(os.path.join(path, "SKILL.md"))
        found[name] = Skill(meta.get("name", name), path, meta)
    return found

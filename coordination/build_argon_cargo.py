"""Export committed source plus a text cargo and SHA-256 inventory for review."""
import hashlib
import json
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "handoff-output"
TEXT_EXTENSIONS = {".rs", ".py", ".md", ".toml", ".json", ".ps1", ".bat", ".yml", ".yaml", ".txt", ".html", ".css", ".js"}


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def main():
    OUT.mkdir(exist_ok=True)
    commit = git("rev-parse", "HEAD").decode().strip()
    prefix = f"teledra-v2-argon-{commit[:12]}"
    archive = OUT / f"{prefix}.zip"
    cargo = OUT / f"{prefix}-cargo.txt"
    manifest = {"commit": commit, "files": []}
    names = git("ls-tree", "-r", "--name-only", "-z", "HEAD").decode().split("\0")
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle, cargo.open("w", encoding="utf-8", newline="\n") as text:
        text.write(f"TELEDRA V2 — GEMINI ARGON REVIEW CARGO\nCommit: {commit}\nStart with coordination/GEMINI_ARGON_REVIEW.md\n")
        # Put the review guide first; remaining paths are deterministic.
        names = sorted(filter(None, names), key=lambda p: (p != "coordination/GEMINI_ARGON_REVIEW.md", p))
        for name in names:
            data = git("show", f"HEAD:{name}")
            bundle.writestr(f"{prefix}/{name}", data)
            manifest["files"].append({"path": name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
            path = Path(name)
            if path.suffix in TEXT_EXTENSIONS or path.name in {".gitignore", "Cargo.lock"}:
                try:
                    content = data.decode("utf-8")
                except UnicodeDecodeError:
                    continue
                text.write(f"\n\n===== BEGIN FILE: {name} =====\n{content}\n===== END FILE: {name} =====\n")
    manifest["artifacts"] = [{"path": p.name, "bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in (archive, cargo)]
    inventory = OUT / f"{prefix}-manifest.json"
    inventory.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    with zipfile.ZipFile(archive) as bundle:
        assert bundle.testzip() is None
        assert len(bundle.namelist()) == len(manifest["files"])
    print(json.dumps({"commit": commit, "files": len(manifest["files"]), "archive": str(archive), "cargo": str(cargo), "manifest": str(inventory)}, indent=2))


if __name__ == "__main__":
    main()

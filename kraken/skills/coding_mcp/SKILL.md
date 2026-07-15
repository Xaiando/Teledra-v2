---
name: coding_mcp
timeout_s: 120
max_children: 0
harness: verify_coding_mcp
---

# coding_mcp - safe local coding tools

Deterministic coding-tool surface for the Kraken workers. This is intentionally
MCP-shaped but local: inspect files, search, list trees, run Python compile/test
checks, and view git status without giving generated code arbitrary shell access.

Inputs are JSON:

```json
{"op": "tree", "path": ".", "max_files": 80}
{"op": "read", "path": "games/animated_game/animated_game.py"}
{"op": "search", "path": ".", "pattern": "TODO"}
{"op": "py_compile", "path": "games/animated_game"}
{"op": "run_tests", "path": "games/animated_game"}
{"op": "git_status", "path": "."}
```

Allowed paths are confined to Kraken root and the operator workspace.

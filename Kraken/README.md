# Kraken v2

Kraken is the standalone, local research and productivity taskforce. It keeps
the V1 operator surface while replacing Python-global state and raw model
shell access with typed Rust contracts. The only model adapter is local Ollama
`qwen2.5:7b`.

Runtime state defaults to `%LOCALAPPDATA%\TeledraV2\Kraken`; override it with
`TELEDRA_KRAKEN_ROOT`. The free-rein workspace defaults beneath that data root
and can be changed with `KRAKEN_WORKSPACE`.

## Commands

```powershell
cargo run -p teledra-kraken --bin kraken -- add research_local "question"
cargo run -p teledra-kraken --bin kraken -- run 25
cargo run -p teledra-kraken --bin kraken -- daemon
cargo run -p teledra-kraken --bin kraken -- worker kraken-worker-1 D:\SomeProject
cargo run -p teledra-kraken --bin kraken -- status
cargo run -p teledra-kraken --bin kraken -- watch
cargo run -p teledra-kraken --bin kraken -- skills
cargo run -p teledra-kraken --bin kraken -- graduate-games
cargo run -p teledra-kraken --bin kraken -- compact
```

`add` also accepts `--input-file PATH` and `@PATH` so structured forge
contracts do not have to survive shell quoting.

## Durable execution

- Append-only, last-state-wins JSONL queue with atomic FIFO claims, torn-line
  recovery, stale-job reaping, compaction, and bounded feedback.
- Recursion depth 3, at most 5 children per parent, two normal attempts, and up
  to five contract-declared forge attempts (three by default for V1 payloads).
- Dedicated Hub mode at `127.0.0.1:3838`: exactly-once mission claims,
  duplicate in-flight suppression, worker presence/signals, and job mirroring
  into the Hub task board. A missing Hub never stops local work.
- Stale queue, mission, task-map, and worker PID locks recover without manual
  deletion; live duplicate workers are refused.

## Skills

- `research_local`, `research_web`, `research_synth`, `research_fanout`:
  bounded evidence retrieval, local-to-web escalation, at least three fetched
  web sources for a search, exact `[L#]`/`[S#]` labels, and citation coverage
  for every prose paragraph or bullet. Failed model citation rewrites fall
  back to directly quoted, query-ranked source excerpts.
- `coding_mcp`: confined tree/read/search, Python compilation, test, and git
  status operations. It exposes no raw shell selected by model output.
- `code_forge`: immutable task digest, isolated candidate tree, protected
  paths, forbidden patterns, required outputs, repeated mechanical gates, and
  atomic publication only after success. Repair attempts retain the candidate
  and receive the prior verifier evidence; the complete verified diff is
  published, including files created on an earlier attempt.
- `prod_digest`, `prod_vault`, `introspect`: bounded source digests,
  evergreen notes, and evidence-backed operational self-audit.
- `pixelfork`: exact-host provenance import with an optional verified forge
  child.
- `graduate_games`: central inventory plus agreeing local declaration,
  static safety checks, exact artifact hashing, and a real headless
  Chrome/Edge probe. The probe clicks Play, instruments animation and audio,
  verifies Beast v2 profile identity/snapshot fields, performs a
  profile-specific action, proves state changed, and rejects an artifact that
  changes during inspection.
- `swarm_architect`: validates and schedules at most five registered typed
  child skills; it cannot recursively schedule itself.

## Verification

```powershell
cargo test -p teledra-kraken
cargo clippy -p teledra-kraken --all-targets -- -D warnings
```

The maintained inventory in `game_inventory.json` describes the V1 production
campaign without copying generated game artifacts into V2. Point
`KRAKEN_WORKSPACE` at a workspace containing `games/` to run the acceptance
campaign.

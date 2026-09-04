# FrankenSwarm v2

FrankenSwarm is the standalone local OLMo merge laboratory and role-separated
agent blackboard. It retains root-court provenance composition while replacing
the V1 product's mocked science and API stubs with deterministic, fail-closed
owners.

Generated state defaults to `%LOCALAPPDATA%\TeledraV2\Frankenswarm`; source
configs and evaluation suites remain under this crate. Override data with
`FRANKENSWARM_DATA_DIR` and the SQLite path with `FS_DB_PATH`.

## Commands

```powershell
cargo run -p teledra-frankenswarm --bin frankenswarm -- preflight
cargo run -p teledra-frankenswarm --bin frankenswarm -- lock-models
cargo run -p teledra-frankenswarm --bin frankenswarm -- download-models
cargo run -p teledra-frankenswarm --bin frankenswarm -- autopsy
cargo run -p teledra-frankenswarm --bin frankenswarm -- autopsy --weights
cargo run -p teledra-frankenswarm --bin frankenswarm -- tournament --seeds 17,29,43
cargo run -p teledra-frankenswarm --bin frankenswarm -- build-candidate --config configs/merge/ties_d050.yaml --seed 17
cargo run -p teledra-frankenswarm --bin frankenswarm -- eval-baselines --endpoint http://127.0.0.1:8001/ --model MODEL
cargo run -p teledra-frankenswarm --bin frankenswarm -- select-winner
cargo run -p teledra-frankenswarm --bin frankenswarm -- serve-command
cargo run -p teledra-frankenswarm --bin frankenswarm -- init-db
cargo run -p teledra-frankenswarm --bin frankenswarm -- runtime --addr 127.0.0.1:8000
cargo run -p teledra-frankenswarm --bin frankenswarm -- sandbox-build
cargo run -p teledra-frankenswarm --bin frankenswarm -- sandbox --workspace PATH -- COMMAND
```

Planning commands are dry by default. Large merge execution additionally
requires `--execute` and `FRANKENSWARM_ENABLE_MERGE=1`.

## Reproducibility and gates

- Model names are resolved to immutable 40-character Hugging Face revisions.
  Downloads produce a lock-digest-bound snapshot manifest.
- Autopsy downloads immutable config/tokenizer metadata, normalizes equivalent
  JSON/token encodings, compares architecture and exact token IDs, and scopes
  merge safety to phase-0 science donors. It never invents tensor statistics.
- Merge execution requires a matching safe autopsy, completed streamed tensor
  analysis, matching download manifest, and existing local snapshot weights.
  A candidate-specific YAML is rendered with canonical local snapshot paths,
  so mergekit cannot silently resolve a mutable repository head.
- The current locked OLMo campaign correctly fails metadata authorization:
  the three phase-0 RL-Zero donors use a different token at vocabulary ID
  100256 than the base. This is a discovered upstream incompatibility, not a
  disabled code path.
- Tournament plans validate method, donors, base, tokenizer source, dtype,
  config digest, lock digest, and deterministic seed. `--allow-crimes` is
  never emitted.
- Evaluation uses bounded local JSONL suites and accepts only loopback HTTP
  OpenAI-compatible endpoints. Winner selection enforces a minimum score in
  every category.

## Swarm runtime and sandbox

The SQLite blackboard uses WAL, foreign keys, atomic immediate transactions,
dependency-aware claims, expiring leases, an explicit task state machine,
events, artifacts, claims, memory, and evaluation tables. The Axum API binds
only to loopback and exposes project/task create, read, claim, and transition
operations.

Generated code runs only through a Podman/Docker container with no network,
read-only root, dropped capabilities, no-new-privileges, PID/CPU/memory limits,
a bounded temporary filesystem, and bounded output/time. The host shell is
not an accepted runtime.

## Verification

```powershell
cargo test -p teledra-frankenswarm
cargo clippy -p teledra-frankenswarm --all-targets -- -D warnings
```

The current machine has NVIDIA tooling but not a Hugging Face downloader,
mergekit, or Podman. Heavy model downloads, tensor analysis, merging, serving,
and container execution therefore remain provisioning-dependent live gates;
the product reports those facts instead of fabricating success.

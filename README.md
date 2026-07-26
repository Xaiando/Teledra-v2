# Teledra V2

Teledra V2 is a clean, modular rewrite of the sovereign court. It uses one
language model only: `qwen2.5:7b` through the local Ollama service.

The old `D:\Teledra` tree is a read-only source of behavior and lessons. Code is
not copied into V2 merely because it existed in V1. Every migrated feature gets
a clear owner, a small contract, and tests.

## Current working slice

- `Teledra/Brain`: the only model boundary; it rejects any model other than
  `qwen2.5:7b`.
- `Agents`: independently editable court-member profiles.
- `Court`: role selection and bounded, role-local conversation history.
- `Teledra`: a native terminal court shell.

The remaining top-level folders are explicit migration boundaries. Their
readmes describe what belongs there and what must not leak across the boundary.

## Run

Ollama must be running with `qwen2.5:7b` installed.

For normal use, run:

```powershell
.\Teledra.bat
```

For development:

```powershell
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
& $cargo run -p teledra
```

Useful commands inside the court:

```text
/roles
/role queen
/clear
/health
/quit
```

To use a non-default Ollama address, set `TELEDRA_OLLAMA_URL`. The model name is
deliberately not configurable.

## Verify

```powershell
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
& $cargo fmt --all -- --check
& $cargo test --workspace
& $cargo clippy --workspace --all-targets -- -D warnings
& $cargo run -p teledra -- --check
```

See [MIGRATION.md](MIGRATION.md) for the migration order and rejection rules.

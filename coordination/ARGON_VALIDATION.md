# Argon snapshot validation — 14 October 2026

Validation ran against the local modular V2 snapshot on Windows. Application
source was preserved for inspection; this handoff did not repair unrelated
behavior or rewrite assertions to hide failures.

| Check | Result |
| --- | --- |
| Rust workspace, default model environment | `cargo test --workspace --locked --no-fail-fast`: exit 0, 747 passed, 0 failed, 21 ignored across 47 test binaries/doc-test groups |
| Rust workspace, machine environment (`TELEDRA_MODEL=qwen2.5-coder:14b`) | Same command: exit 101, 743 passed, 4 failed, 21 ignored; all failures compare the configured model name to hard-coded `qwen2.5:7b` |
| Offline YouTube Python tests | 16 passed |
| Python music score tests | 8 passed, 1 skipped |
| Python source syntax | All 123 then-tracked Python files parsed successfully with Python AST |
| Credential pattern screening | No matches in the then-tracked 468-file snapshot for private-key headers, GitHub/Google/OpenAI token patterns, or long literal password/secret/token assignments; this is a limited pattern check |
| Large-file screening | No tracked file above 50 MiB; source/assets totaled approximately 29.1 MiB before handoff docs and evidence |
| Whitespace check | Reports trailing whitespace in preserved downloaded HTML provenance pages; those snapshots were not normalized |

The default-model run removed `TELEDRA_MODEL` only from the test subprocess
environment. The operator's configured environment was preserved.

The four non-default-model failures are:

- `python_composition::tests::exact_edit_uses_real_schema_and_runtime_identity_without_a_prose_critic` (application binary and `live_python_evolution` integration target).
- `tests::spawn_turn_routes_accepted_python_edits_through_the_artifact_schema` (application binary).
- `tests::slow_private_audit_returns_the_complete_draft` (`teledra-brain`).

This exposes test assumptions around the supported model override. It does
not establish whether either model produces better live behavior.

Full output is included in [argon-evidence](argon-evidence/):

- [Default-model workspace run](argon-evidence/argon-default-model-tests.txt)
- [Configured-model workspace run](argon-evidence/argon-workspace-all-tests.txt)
- [Focused Python checks](argon-evidence/argon-python-tests.txt)

Hardware-dependent ignored checks, fresh live Ollama conversations, TTS voice
quality, actual listening quality, external services, the full FlyComposer
suite, and a fresh release build were not repeated for this source handoff.
The generated ZIP is checked for CRC integrity and entry count by its export
script; the accompanying manifest provides per-file and artifact SHA-256.

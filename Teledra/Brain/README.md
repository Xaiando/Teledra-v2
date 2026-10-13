# Teledra Brain

This is the entire model boundary for V2.

- Transport: local Ollama native API.
- Model: exactly `qwen2.5:7b`.
- No provider abstraction.
- No API keys.
- No alternate chat or code model.
- No court personality or tool policy.

If another subsystem needs inference, it submits messages through this crate.

Requests explicitly budget 16,384 context tokens and 4,096 output tokens.
Output-limit truncation is reported; ordinary completion gets one compact retry.
Specialist refinement retains the original task and identity context.
`complete_structured` accepts a caller-owned JSON schema through Ollama's
[structured output format](https://docs.ollama.com/capabilities/structured-outputs).
The Music crate owns the score schema and native validation.

`complete_structured_with_budget` uses neutral JSON artifact instructions and a
caller-owned total deadline. PyMusic has its own constrained edit/source boundary;
it bypasses the ordinary spoken-answer critic and refiner. The generic refiner
also preserves `PYTHON_MUSIC_EDIT` tags if one appears in an ad hoc answer.

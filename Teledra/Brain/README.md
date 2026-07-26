# Teledra Brain

This is the entire model boundary for V2.

- Transport: local Ollama native API.
- Model: exactly `qwen2.5:7b`.
- No provider abstraction.
- No API keys.
- No alternate chat or code model.
- No court personality or tool policy.

If another subsystem needs inference, it submits messages through this crate.


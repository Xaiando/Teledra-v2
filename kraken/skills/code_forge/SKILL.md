---
name: code_forge
timeout_s: 1500
max_children: 0
harness: verify_code
---

# code_forge - Ornith repair loop for small code artifacts

**Input:** either a plain-language coding task, or JSON:

```json
{
  "task": "Write add(a, b) returning the sum.",
  "filename": "math_utils.py",
  "tests": "import math_utils; assert math_utils.add(2, 3) == 5",
  "seed_code": "def add(a, b):\n    return a - b\n",
  "dir": "projects/example",
  "quality": "beast",
  "verify_only": false
}
```

**Output:** a code file under the job workdir. If `tests` is supplied, it is
written to `test_<module>.py` and returned to `verify_code`.

When `dir` targets the shared workspace, generation and repairs happen in the
job workdir first. Kraken publishes to the workspace only after verification
passes. Set `verify_only: true` to stage and independently certify an existing
workspace artifact without asking the model to rewrite it. `quality: beast` on
browser games also requires the headless runtime and gameplay evidence contract.

**Models (coupled):** Primary generation uses `qwen2.5:7b` (stronger general reasoning + instruction following to assist the overall process). Coupled with Ornith (`hf.co/deepreinforce-ai/Ornith-1.0-9B-GGUF:Q4_K_M`) as the code specialist, used by default on repair passes. Both served locally via Ollama. You can force a single model for a job by including `"model": "qwen2.5:7b"` (or `"ornith"`) in the input payload. 32K context supported for rich game artifacts.

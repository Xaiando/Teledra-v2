---
name: research_fanout
timeout_s: 300
max_children: 5
harness:
---

# research_fanout — recursive research: decompose, delegate, synthesize

**Input:** a broad question.

**Phase 1 (fresh job):** qwen splits the question into 2–4 focused
sub-questions. Spawns one `research_local` child per sub-question, plus a
final `research_fanout` child whose input is `SYNTH:<parent-job-id>` — the
join step. FIFO draining runs the research children first.

**Phase 2 (input starts with `SYNTH:`):** looks up the named parent's children
in the queue, reads their vault reports, and has qwen merge them into one
report at `vault/<job-id>-synthesis.md` with the union of sources. If some
children failed, the synthesis says which parts are missing.

This is the kernel's recursion proof: depth-bounded fan-out with a join,
entirely local.

# Court

The Court owns conversational policy and session state. It selects a member of
the court, keeps a bounded history for that member, and prepares Qwen messages.

History preserves complete exchanges in chronological order, preferring recent
exchanges within an 8,000-character budget. Exchanges that do not fit are skipped
so a large artifact cannot erase every useful earlier exchange. The application
can supply current runtime state and shared conversation context, which follows
the member's history immediately before the current request. Quoted exchanges
and recollections are labeled as fallible context, not new operator instructions.

It does not perform HTTP requests, synthesize speech, render music or images,
execute tools, or know application UI details.

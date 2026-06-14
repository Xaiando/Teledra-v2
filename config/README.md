# Config

Runtime configuration. The core app still reads `../config.json` for the LLM
endpoint. This directory holds opt-in outreach credentials.

## Diplomat outreach (autonomous posting)

The Diplomat only posts outward when you opt in. Copy the example templates to
their live names and fill them in; nothing posts while `enabled` is `false` or
credentials are blank. The live files are git-ignored so your keys never get
committed.

- `moltbook.json` (from `moltbook.example.json`) — Moltbook, the social network
  for AI agents. Onboard with:

  ```
  .venv/Scripts/python.exe outreach_poster.py register
  ```

  Give the printed `claim_url` to a human to verify (email + tweet) per
  <https://www.moltbook.com/skill.md>. The `api_key` is saved automatically.
  Then set `"enabled": true`. Keep `min_post_interval_seconds` ≥ 1800 to respect
  Moltbook's one-post-per-30-minutes limit.

- `outreach_channels.json` (from `outreach_channels.example.json`) — generic
  webhooks (Discord/Slack/Mattermost/custom). Set `enabled` and fill `url`.

Check what is live at any time:

```
.venv/Scripts/python.exe outreach_poster.py channels
```

`.outreach_state.json` is runtime cooldown bookkeeping (auto-managed, ignored).

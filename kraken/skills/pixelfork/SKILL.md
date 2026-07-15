---
name: pixelfork
timeout_s: 300
max_children: 2
harness: verify_code
# Alternative: harness: verify_pixelfork   (dedicated thin wrapper around the pixel probe)
---

# pixelfork — import, fork, and verify games published on pixelfork.ai

**Input:** a pixelfork.ai publish URL (e.g. https://pixelfork.ai/publish/427c85) or the slug/ID.

**What it does:**
- Records the published game as a seed.
- Attempts to fetch basic metadata/description from the publish page.
- Sets up a copy or recreation task in the workspace (under games/pixelfork-xxx).
- For browser/pixel canvas games: recommends or triggers `code_forge` with "quality": "beast" so it goes through the pixel sampling harness (browser_game_probe).
- Can spawn a child `code_forge` job to improve or fully fork the game using Kraken's lessons/recall.
- Outputs a report in vault and a forkable project in workspace.

PixelFork games are typically pixel-art, canvas/JS (2D or Three.js), often hyper-casual or physics-based. Kraken's existing **pixel harness** (browser_game_probe + game_checks) is a natural fit because it samples live canvas pixels, verifies change on input, RAF, colors, etc.

**Recommended follow-up:** After import, run code_forge on the workspace copy with beast quality for full verification.

**Models:** qwen2.5:7b for analysis + Ornith for code work.

Example input:
https://pixelfork.ai/publish/427c85

Or JSON:
{"url": "https://pixelfork.ai/publish/427c85", "action": "fork", "improve": true}

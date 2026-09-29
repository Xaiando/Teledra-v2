# Local Strudel workspace

This compatibility surface validates and plays Strudel patterns. Court Synth is
Teledra's primary composition surface (`court_synthesizer.py`).

From the repository root, install the locked dependencies:

```powershell
npm ci --prefix strudel_app
node strudel_app/app.mjs validate strudel_app/depth_fixture.strudel
```

Playback uses the repository's `.venv/Scripts/python.exe` and `teledra_synth.py`.
Runtime state and `node_modules` are excluded from Git.

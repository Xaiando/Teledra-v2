# Sidecars

Owns optional device and external-system adapters that are inappropriate for
the native core: microphone transcription, desktop capture/OCR, somatic host
signals, stream chat, approved outreach, read-only treasury feeds, MCP stdio,
and YouTube transcripts.

The Rust manager starts one bounded job, writes JSON input, continuously
drains stdout/stderr, applies a timeout and output ceiling, and kills/reaps the
child on failure. Scripts print exactly one JSON response. Unavailable optional
packages disable only their capability.

YouTube ingestion accepts standard, short, mobile, Shorts, live, and embed video
links. It prefers English captions and falls back to the video's available
language. Long transcripts include clearly marked excerpts from throughout the
recording, bounded to 6,000 characters so the full report fits the court's context. If captions are unavailable but YouTube
provides a title/channel, the response explicitly identifies this as metadata
only; it is not evidence of the video's contents. Network failures without any
video metadata return an actionable error. JSON output is escaped for reliable
Unicode transport through Windows pipes.

Run the YouTube regressions with the configured Python interpreter:

```powershell
.venv\Scripts\python.exe -m unittest discover -s Sidecars\scripts -p test_youtube.py
```

Optional packages:

```powershell
python -m pip install -r Sidecars\requirements-optional.txt
```

On Windows, microphone capture falls back to the built-in `System.Speech`
recognizer. Other platforms, or users preferring Google's recognizer, need a
working SpeechRecognition and PortAudio/PyAudio install. OCR needs the
Tesseract executable visible to `pytesseract`; desktop capture still works
when OCR is unavailable.

On Windows the standard Winget package is sufficient, even when its installer
does not add Tesseract to `PATH`:

```powershell
winget install --id UB-Mannheim.TesseractOCR --exact
```

For a custom installation, set `TELEDRA_TESSERACT` to `tesseract.exe`.

MCP servers are off by default. `TELEDRA_MCP_CONFIG` must point to JSON with an
explicitly enabled server, for example:

```json
{
  "servers": {
    "example": {
      "enabled": true,
      "command": "python",
      "args": ["server.py"]
    }
  }
}
```

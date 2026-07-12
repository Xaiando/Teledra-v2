"""Headless runtime probe for self-contained browser games.

Static source checks catch structure. This probe catches the more important
class of failures: a page that parses, accepts Play, and then crashes or never
advances. It uses the browser already installed on Windows, so Kraken does not
need Playwright or another runtime dependency.
"""

from __future__ import annotations

import html as html_lib
import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any


VIRTUAL_TIME_MS = 4200
PROCESS_TIMEOUT_S = 25


BOOTSTRAP = r"""
<script id="kraken-probe-bootstrap">
(() => {
  const probe = window.__krakenProbe = {
    errors: [], consoleErrors: [], rafCount: 0, audioStarts: 0
  };
  const remember = value => {
    if (probe.errors.length < 20) probe.errors.push(String(value));
  };
  window.addEventListener('error', event => remember(event.error?.stack || event.message));
  window.addEventListener('unhandledrejection', event => remember(event.reason?.stack || event.reason));

  const originalError = console.error.bind(console);
  console.error = (...args) => {
    if (probe.consoleErrors.length < 20) probe.consoleErrors.push(args.map(String).join(' '));
    originalError(...args);
  };

  window.requestAnimationFrame = callback => window.setTimeout(() => {
    probe.rafCount += 1;
    try {
      return callback(performance.now());
    } catch (error) {
      remember(error?.stack || error);
      throw error;
    }
  }, 16);
  window.cancelAnimationFrame = handle => window.clearTimeout(handle);

  class FakeParam {
    setValueAtTime() { return this; }
    linearRampToValueAtTime() { return this; }
    exponentialRampToValueAtTime() { return this; }
    setTargetAtTime() { return this; }
    cancelScheduledValues() { return this; }
  }
  class FakeNode {
    connect() { return this; }
    disconnect() {}
  }
  class FakeOscillator extends FakeNode {
    constructor() { super(); this.frequency = new FakeParam(); this.detune = new FakeParam(); this.type = 'sine'; }
    start() { probe.audioStarts += 1; }
    stop() {}
  }
  class FakeGain extends FakeNode {
    constructor() { super(); this.gain = new FakeParam(); }
  }
  class FakeBufferSource extends FakeNode {
    constructor() { super(); this.playbackRate = new FakeParam(); this.detune = new FakeParam(); this.loop = false; }
    start() { probe.audioStarts += 1; }
    stop() {}
  }
  class FakeAudioContext {
    constructor() { this.currentTime = 0; this.state = 'running'; this.sampleRate = 44100; this.destination = new FakeNode(); }
    createOscillator() { return new FakeOscillator(); }
    createGain() { return new FakeGain(); }
    createBufferSource() { return new FakeBufferSource(); }
    createBiquadFilter() { const node = new FakeNode(); node.frequency = new FakeParam(); node.Q = new FakeParam(); return node; }
    createDynamicsCompressor() { return new FakeNode(); }
    createStereoPanner() { const node = new FakeNode(); node.pan = new FakeParam(); return node; }
    createBuffer() { return {}; }
    resume() { this.state = 'running'; return Promise.resolve(); }
    suspend() { this.state = 'suspended'; return Promise.resolve(); }
    close() { this.state = 'closed'; return Promise.resolve(); }
  }
  try {
    Object.defineProperty(window, 'AudioContext', { configurable: true, value: FakeAudioContext });
    Object.defineProperty(window, 'webkitAudioContext', { configurable: true, value: FakeAudioContext });
  } catch (error) {
    remember('audio instrumentation failed: ' + error);
  }
})();
</script>
"""


DRIVER = r"""
<script id="kraken-probe-driver">
window.addEventListener('load', async () => {
  const probe = window.__krakenProbe;
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  const clone = value => {
    try { return JSON.parse(JSON.stringify(value)); } catch (_) { return null; }
  };
  const snapshot = () => {
    const api = window.__KRAKEN_BEAST__;
    if (!api || typeof api.snapshot !== 'function') return null;
    try { return clone(api.snapshot()); } catch (error) {
      probe.errors.push('beast snapshot failed: ' + (error?.stack || error));
      return null;
    }
  };
  const key = (type, code) => window.dispatchEvent(new KeyboardEvent(type, {
    code, key: code === 'Space' ? ' ' : code.replace(/^Key/, ''), bubbles: true
  }));
  const isVisible = element => {
    if (!element) return false;
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity || 1) > 0 && rect.width > 0 && rect.height > 0;
  };
  const sampleCanvas = () => {
    const canvas = document.querySelector('canvas');
    if (!canvas) return null;
    try {
      const context = canvas.getContext('2d');
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
      const stride = Math.max(4, Math.floor(pixels.length / 12000 / 4) * 4);
      const colors = new Set();
      let hash = 2166136261;
      let opaque = 0;
      for (let i = 0; i < pixels.length; i += stride) {
        const packed = `${pixels[i]},${pixels[i + 1]},${pixels[i + 2]},${pixels[i + 3]}`;
        if (colors.size < 256) colors.add(packed);
        if (pixels[i + 3] > 20) opaque += 1;
        hash ^= pixels[i]; hash = Math.imul(hash, 16777619);
        hash ^= pixels[i + 1]; hash = Math.imul(hash, 16777619);
        hash ^= pixels[i + 2]; hash = Math.imul(hash, 16777619);
      }
      return { width: canvas.width, height: canvas.height, colors: colors.size, opaque, hash: hash >>> 0 };
    } catch (error) {
      probe.errors.push('canvas sampling failed: ' + (error?.stack || error));
      return null;
    }
  };
  const findButton = pattern => Array.from(document.querySelectorAll('button')).find(button => pattern.test(button.textContent || '') && isVisible(button));

  const urlParams = new URLSearchParams(window.location.search);
  const profileName = urlParams.get('profile') || 'platformer';
  const sessionType = urlParams.get('session') || 'finite';
  const contractVer = parseInt(urlParams.get('version') || '1');

  const api = window.__KRAKEN_BEAST__;
  const isV2 = contractVer >= 2 && api && api.version >= 2;

  const report = { clickedPlay: false, overlayHiddenAfterStart: false, telemetry: {} };
  await sleep(60);
  const play = document.querySelector('#playBtn, #startBtn, #play-btn, [data-action="play"]') || findButton(/play|start/i);
  if (play) {
    play.click();
    report.clickedPlay = true;
  }
  await sleep(180);
  const overlay = document.querySelector('#overlay, #startScreen, #start-screen, .start-screen, .overlay');
  report.overlayHiddenAfterStart = !isVisible(overlay);
  report.canvasStart = sampleCanvas();
  report.telemetry.initial = snapshot();

  if (profileName === 'snake') {
    for (const code of ['ArrowDown', 'ArrowRight']) {
      key('keydown', code);
      await sleep(100);
      key('keyup', code);
    }
    report.canvasAfterRight = sampleCanvas();
    report.telemetry.afterRight = snapshot();
    if (isV2 && api && typeof api.action === 'function') {
      try { await Promise.resolve(api.action('feed')); } catch (e) { probe.errors.push('beast feed failed: ' + e); }
      await sleep(100);
      report.telemetry.afterFeed = snapshot();

      try { await Promise.resolve(api.action('collide')); } catch (e) { probe.errors.push('beast collide failed: ' + e); }
      await sleep(100);
      report.telemetry.afterDamage = snapshot();
      report.telemetry.transitions = [snapshot()];
    }
  } else {
    for (const code of ['ArrowRight', 'KeyD']) key('keydown', code);
    await sleep(280);
    report.canvasAfterRight = sampleCanvas();
    report.telemetry.afterRight = snapshot();
    for (const code of ['ArrowRight', 'KeyD']) key('keyup', code);

    for (const code of ['Space', 'ArrowUp', 'KeyW']) key('keydown', code);
    await sleep(90);
    report.telemetry.duringJump = snapshot();
    for (const code of ['Space', 'ArrowUp', 'KeyW']) key('keyup', code);
    await sleep(160);
    report.canvasAfterJump = sampleCanvas();
    report.telemetry.afterJump = snapshot();

    if (isV2 && api && typeof api.action === 'function') {
      try { await Promise.resolve(api.action('damage')); } catch (e) { probe.errors.push('beast damage failed: ' + e); }
      await sleep(100);
      report.telemetry.afterDamage = snapshot();

      try { await Promise.resolve(api.action('advance')); } catch (e) { probe.errors.push('beast advance failed: ' + e); }
      await sleep(100);
      report.telemetry.transitions = [snapshot()];
    } else if (api && typeof api.damage === 'function') {
      try { api.damage(); } catch (error) { probe.errors.push('beast damage failed: ' + (error?.stack || error)); }
      await sleep(100);
      report.telemetry.afterDamage = snapshot();

      if (typeof api.completeLevel === 'function') {
        try { api.completeLevel(); } catch (error) { probe.errors.push('beast completeLevel failed: ' + (error?.stack || error)); }
        await sleep(100);
        report.telemetry.transitions = [snapshot()];
      }
    }
  }

  report.canvasFinal = sampleCanvas();
  report.rafCount = probe.rafCount;
  report.audioStarts = probe.audioStarts;
  report.errors = probe.errors;
  report.consoleErrors = probe.consoleErrors;
  report.beastApi = Boolean(api && typeof api.snapshot === 'function' && (isV2 ? typeof api.action === 'function' : (typeof api.damage === 'function' && typeof api.completeLevel === 'function')));

  const output = document.createElement('pre');
  output.id = 'kraken-runtime-report';
  output.textContent = JSON.stringify(report);
  document.body.replaceChildren(output);
});
</script>
"""


def find_browser() -> str | None:
    candidates = [
        shutil.which("msedge"),
        shutil.which("chrome"),
        os.path.join(os.environ.get("PROGRAMFILES(X86)", ""), "Microsoft", "Edge", "Application", "msedge.exe"),
        os.path.join(os.environ.get("PROGRAMFILES", ""), "Microsoft", "Edge", "Application", "msedge.exe"),
        os.path.join(os.environ.get("PROGRAMFILES", ""), "Google", "Chrome", "Application", "chrome.exe"),
        os.path.join(os.environ.get("PROGRAMFILES(X86)", ""), "Google", "Chrome", "Application", "chrome.exe"),
    ]
    for candidate in candidates:
        if candidate and os.path.isfile(candidate):
            return os.path.abspath(candidate)
    return None


def _inject(html: str) -> str:
    head = re.search(r"<head\b[^>]*>", html, re.IGNORECASE)
    if head:
        html = html[:head.end()] + BOOTSTRAP + html[head.end():]
    else:
        html = BOOTSTRAP + html
    body_end = list(re.finditer(r"</body\s*>", html, re.IGNORECASE))
    if body_end:
        pos = body_end[-1].start()
        return html[:pos] + DRIVER + html[pos:]
    return html + DRIVER


def _extract_report(dumped_dom: str) -> dict[str, Any] | None:
    match = re.search(
        r'<pre\b[^>]*id=["\']kraken-runtime-report["\'][^>]*>(.*?)</pre>',
        dumped_dom,
        re.IGNORECASE | re.DOTALL,
    )
    if not match:
        return None
    try:
        return json.loads(html_lib.unescape(match.group(1)))
    except json.JSONDecodeError:
        return None


def _num(snapshot: Any, *path: str) -> float | None:
    value = snapshot
    for key in path:
        if not isinstance(value, dict) or key not in value:
            return None
        value = value[key]
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    return None


def _terminal(snapshot: Any) -> bool:
    if not isinstance(snapshot, dict):
        return False
    return snapshot.get("complete") is True or bool(
        re.search(r"won|win|victory|complete|finished", str(snapshot.get("state", "")), re.IGNORECASE)
    )


def assess(report: dict[str, Any], *, require_beast: bool = False) -> list[str]:
    reasons: list[str] = []
    errors = [str(item) for item in (report.get("errors") or []) if str(item).strip()]
    errors += [str(item) for item in (report.get("consoleErrors") or []) if str(item).strip()]
    if errors:
        reasons.append("browser runtime error after Play: " + " | ".join(dict.fromkeys(errors))[:900])
    if not report.get("clickedPlay"):
        reasons.append("browser runtime probe could not find/click a visible Play or Start button")
    if not report.get("overlayHiddenAfterStart"):
        reasons.append("start overlay remains visible after Play")
    if int(report.get("rafCount") or 0) < 8:
        reasons.append("animation loop did not sustain at least 8 requestAnimationFrame callbacks")

    start = report.get("canvasStart") or {}
    if int(start.get("width") or 0) <= 0 or int(start.get("height") or 0) <= 0:
        reasons.append("runtime probe found no drawable canvas after Play")
    elif int(start.get("colors") or 0) < 4:
        reasons.append("canvas remains visually blank/flat after Play (fewer than 4 sampled colors)")

    frame_hashes = {
        item.get("hash") for item in (
            report.get("canvasStart") or {},
            report.get("canvasAfterRight") or {},
            report.get("canvasFinal") or {},
        ) if item.get("hash") is not None
    }
    if len(frame_hashes) < 2:
        reasons.append("canvas pixels did not change across movement/jump input; animation appears stalled")

    if not require_beast:
        return reasons
    if not report.get("beastApi"):
        reasons.append("beast game contract missing: expose window.__KRAKEN_BEAST__ in ?krakenTest mode")
        return reasons

    telemetry = report.get("telemetry") or {}
    initial = telemetry.get("initial")
    moved = telemetry.get("afterRight")
    jumped = telemetry.get("duringJump")
    damaged = telemetry.get("afterDamage")
    transitions = [item for item in (telemetry.get("transitions") or []) if isinstance(item, dict)]

    x0 = _num(initial, "player", "x")
    x1 = _num(moved, "player", "x")
    if x0 is None or x1 is None or x1 <= x0 + 1:
        reasons.append("beast telemetry: holding Right did not advance player.x")
    y0 = _num(moved, "player", "y")
    y1 = _num(jumped, "player", "y")
    vy1 = _num(jumped, "player", "vy")
    if y0 is None or y1 is None or vy1 is None or not (y1 < y0 - 0.5 or vy1 < -0.25):
        reasons.append("beast telemetry: Space/Up did not produce an upward jump")
    lives0 = _num(telemetry.get("afterJump"), "lives")
    lives1 = _num(damaged, "lives")
    if lives0 is None or lives1 is None or lives1 >= lives0:
        reasons.append("beast telemetry: damage() did not reduce the real lives counter")
    if int(report.get("audioStarts") or 0) < 1:
        reasons.append("no Web Audio source started during Play, jump, or damage")
    if not transitions:
        reasons.append("beast telemetry: completeLevel() produced no progression snapshots")
    elif not _terminal(transitions[-1]):
        reasons.append("beast telemetry: repeated real exit transitions never reached a win state")
    return reasons


def assess_structured(
    report: dict[str, Any],
    *,
    expected_profile: str,
    session: str,
    contract_version: int,
) -> list[str]:
    reasons: list[str] = []
    errors = [str(item) for item in (report.get("errors") or []) if str(item).strip()]
    errors += [str(item) for item in (report.get("consoleErrors") or []) if str(item).strip()]
    if errors:
        reasons.append("browser runtime error after Play: " + " | ".join(dict.fromkeys(errors))[:900])
    if not report.get("clickedPlay"):
        reasons.append("browser runtime probe could not find/click a visible Play or Start button")
    if not report.get("overlayHiddenAfterStart"):
        reasons.append("start overlay remains visible after Play")
    if int(report.get("rafCount") or 0) < 8:
        reasons.append("animation loop did not sustain at least 8 requestAnimationFrame callbacks")

    start = report.get("canvasStart") or {}
    if int(start.get("width") or 0) <= 0 or int(start.get("height") or 0) <= 0:
        reasons.append("runtime probe found no drawable canvas after Play")
    elif int(start.get("colors") or 0) < 4:
        reasons.append("canvas remains visually blank/flat after Play (fewer than 4 sampled colors)")

    frame_hashes = {
        item.get("hash") for item in (
            report.get("canvasStart") or {},
            report.get("canvasAfterRight") or {},
            report.get("canvasFinal") or {},
        ) if item.get("hash") is not None
    }
    if len(frame_hashes) < 2:
        reasons.append("canvas pixels did not change; animation appears stalled")

    if not report.get("beastApi"):
        reasons.append("beast game contract missing: expose window.__KRAKEN_BEAST__ matching the resolved profile")
        return reasons

    telemetry = report.get("telemetry") or {}
    initial = telemetry.get("initial")
    moved = telemetry.get("afterRight")
    damaged = telemetry.get("afterDamage")
    transitions = [item for item in (telemetry.get("transitions") or []) if isinstance(item, dict)]

    if expected_profile == "platformer":
        x0 = _num(initial, "actor", "x") or _num(initial, "player", "x")
        x1 = _num(moved, "actor", "x") or _num(moved, "player", "x")
        if x0 is None or x1 is None or x1 <= x0 + 0.1:
            reasons.append("beast telemetry: holding Right did not advance actor.x/player.x")
        lives0 = _num(initial, "metrics", "lives") or _num(initial, "lives")
        lives1 = _num(damaged, "metrics", "lives") or _num(damaged, "lives")
        if lives0 is None or lives1 is None or lives1 >= lives0:
            reasons.append("beast telemetry: damage action did not reduce lives")
        if not transitions:
            reasons.append("beast telemetry: advance action produced no progression snapshots")
    elif expected_profile == "snake":
        y0 = _num(initial, "actor", "y") or _num(initial, "player", "y")
        y1 = _num(moved, "actor", "y") or _num(moved, "player", "y")
        if y0 is None or y1 is None or y1 == y0:
            reasons.append("beast telemetry: direction change did not move snake y")
        feed_snap = telemetry.get("afterFeed")
        len0 = _num(initial, "metrics", "length")
        len1 = _num(feed_snap, "metrics", "length")
        score0 = _num(initial, "score")
        score1 = _num(feed_snap, "score")
        if len0 is not None and len1 is not None and len1 <= len0 and score1 <= score0:
            reasons.append("beast telemetry: feed action did not increase length or score")
        if session == "finite":
            state = damaged.get("state") if damaged else None
            if state != "lost" and state != "gameover" and state != "game_over":
                lives0 = _num(initial, "metrics", "lives") or _num(initial, "lives")
                lives1 = _num(damaged, "metrics", "lives") or _num(damaged, "lives")
                if lives0 is None or lives1 is None or lives1 >= lives0:
                    reasons.append("beast telemetry: collide action did not reduce lives or trigger loss")

    if int(report.get("audioStarts") or 0) < 1:
        reasons.append("no Web Audio source started during Play or actions")

    return reasons


def probe(path: str, *, require_beast: bool = False) -> list[str]:
    browser = find_browser()
    if not browser:
        return ["headless browser unavailable for required runtime game verification"] if require_beast else []
    try:
        source = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        return [f"runtime probe cannot read HTML: {exc}"]

    parent = str(Path(path).resolve().parent)
    try:
        with tempfile.TemporaryDirectory(prefix=".kraken-runtime-", dir=parent) as tmp:
            probe_path = Path(tmp) / "probe.html"
            probe_path.write_text(_inject(source), encoding="utf-8", newline="\n")
            profile = Path(tmp) / "profile"
            cmd = [
                browser,
                "--headless=new",
                "--disable-gpu",
                "--disable-extensions",
                "--disable-background-timer-throttling",
                "--disable-renderer-backgrounding",
                "--autoplay-policy=no-user-gesture-required",
                "--allow-file-access-from-files",
                "--no-first-run",
                "--no-default-browser-check",
                f"--user-data-dir={profile}",
                f"--virtual-time-budget={VIRTUAL_TIME_MS}",
                "--run-all-compositor-stages-before-draw",
                "--dump-dom",
                probe_path.as_uri() + "?krakenTest=1",
            ]
            proc = subprocess.run(
                cmd,
                cwd=tmp,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=PROCESS_TIMEOUT_S,
            )
            report = _extract_report(proc.stdout or "")
            if report is None:
                detail = (proc.stderr or proc.stdout or "no browser output").strip()[-600:]
                return [f"headless browser did not produce a runtime report: {detail}"] if require_beast else []
            return assess(report, require_beast=require_beast)
    except subprocess.TimeoutExpired:
        return [f"headless browser runtime probe timed out after {PROCESS_TIMEOUT_S}s"] if require_beast else []
    except OSError as exc:
        return [f"headless browser runtime probe failed: {exc}"] if require_beast else []


def probe_structured(
    path: str,
    *,
    expected_profile: str,
    session: str,
    contract_version: int,
    workdir: str,
) -> dict[str, Any] | None:
    browser = find_browser()
    if not browser:
        return None
    try:
        source = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None

    try:
        tmp_parent = Path(workdir) / "tmp"
        tmp_parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".kraken-runtime-", dir=str(tmp_parent)) as tmp:
            probe_path = Path(tmp) / "probe.html"
            probe_path.write_text(_inject(source), encoding="utf-8", newline="\n")
            profile = Path(tmp) / "profile"
            cmd = [
                browser,
                "--headless=new",
                "--disable-gpu",
                "--disable-extensions",
                "--disable-background-timer-throttling",
                "--disable-renderer-backgrounding",
                "--autoplay-policy=no-user-gesture-required",
                "--allow-file-access-from-files",
                "--no-first-run",
                "--no-default-browser-check",
                f"--user-data-dir={profile}",
                f"--virtual-time-budget={VIRTUAL_TIME_MS}",
                "--run-all-compositor-stages-before-draw",
                "--dump-dom",
                probe_path.as_uri() + f"?krakenTest=1&profile={expected_profile}&session={session}&version={contract_version}",
            ]
            proc = subprocess.run(
                cmd,
                cwd=tmp,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=PROCESS_TIMEOUT_S,
            )
            return _extract_report(proc.stdout or "")
    except Exception:
        return None


__all__ = ["assess", "find_browser", "probe", "assess_structured", "probe_structured"]

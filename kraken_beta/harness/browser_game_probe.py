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


VIRTUAL_TIME_MS = 9500
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
    constructor() { this._createdAt = performance.now(); this.state = 'running'; this.sampleRate = 44100; this.destination = new FakeNode(); }
    get currentTime() { return Math.max(0, (performance.now() - this._createdAt) / 1000); }
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
  const key = (type, code) => {
    const target = document.querySelector('canvas') || document;
    if (target && typeof target.focus === 'function') target.focus();
    return target.dispatchEvent(new KeyboardEvent(type, {
      code, key: code === 'Space' ? ' ' : code.replace(/^Key/, ''), bubbles: true
    }));
  };
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
  const overlay = document.querySelector('#overlay, #startOverlay, #start-overlay, #startScreen, #start-screen, .start-screen, .overlay');
  report.overlayHiddenAfterStart = !isVisible(overlay);
  report.canvasStart = sampleCanvas();
  report.telemetry.initial = snapshot();
  report.audioStartsAfterPlay = probe.audioStarts;

  if (profileName === 'snake') {
    const snakeMetrics = snap => snap && snap.metrics ? snap.metrics : {};
    const snakeSteps = snap => Number(snakeMetrics(snap).steps);
    const snakeDeaths = snap => Number(snakeMetrics(snap).deaths);
    const snakeState = snap => String(snap && snap.state || '').toLowerCase();
    const waitSnake = async (predicate, attempts = 48, interval = 35) => {
      let latest = snapshot();
      for (let i = 0; i < attempts; i++) {
        if (latest && predicate(latest)) return latest;
        await sleep(interval);
        latest = snapshot();
      }
      return latest;
    };
    const invokeSnake = async (name, resultName = name) => {
      try {
        const result = await Promise.resolve(api.action(name));
        report.actionResults[resultName] = result === true;
      } catch (e) {
        report.actionResults[resultName] = false;
        probe.errors.push('beast ' + name + ' failed: ' + e);
      }
    };

    report.actionResults = {};
    const beforeDown = report.telemetry.initial || {};
    key('keydown', 'ArrowDown');
    key('keyup', 'ArrowDown');
    report.telemetry.afterDown = await waitSnake(snap => {
      const direction = snakeMetrics(snap).direction || {};
      return snakeSteps(snap) > snakeSteps(beforeDown) && Number(direction.y) === 1;
    });

    const beforeRight = report.telemetry.afterDown || beforeDown;
    key('keydown', 'ArrowRight');
    key('keyup', 'ArrowRight');
    report.telemetry.afterRight = await waitSnake(snap => {
      const direction = snakeMetrics(snap).direction || {};
      return snakeSteps(snap) > snakeSteps(beforeRight) && Number(direction.x) === 1;
    });
    report.canvasAfterRight = sampleCanvas();

    if (isV2 && api && typeof api.action === 'function') {
      const beforeFeed = report.telemetry.afterRight || beforeRight;
      await invokeSnake('feed');
      report.telemetry.afterFeedSetup = snapshot();
      const feedSetup = report.telemetry.afterFeedSetup || beforeFeed;
      report.telemetry.afterFeed = await waitSnake(
        snap => snakeSteps(snap) > snakeSteps(feedSetup) || ['won', 'complete', 'completed'].includes(snakeState(snap))
      );

      const beforeCollision = report.telemetry.afterFeed || feedSetup;
      await invokeSnake('collide');
      report.telemetry.afterCollisionSetup = snapshot();
      const collisionSetup = report.telemetry.afterCollisionSetup || beforeCollision;
      report.telemetry.afterDamage = await waitSnake(
        snap => snakeDeaths(snap) > snakeDeaths(collisionSetup) || ['lost', 'gameover', 'game_over'].includes(snakeState(snap))
      );

      const restart = document.querySelector('#restartBtn, #restart-btn, #retryBtn, #retry-btn, [data-action="restart"]') || findButton(/restart|retry|again/i);
      report.clickedRestart = Boolean(restart && isVisible(restart));
      if (report.clickedRestart) restart.click();
      report.telemetry.afterRestart = await waitSnake(
        snap => snakeState(snap) === 'playing' && snakeSteps(snap) === 0
      );

      report.telemetry.progressionSetups = [];
      report.telemetry.transitions = [];
      let latest = report.telemetry.afterRestart;
      for (let i = 0; i < 12 && latest && !['won', 'complete', 'completed'].includes(snakeState(latest)); i++) {
        await invokeSnake('feed', 'progression_feed_' + i);
        const setup = snapshot();
        report.telemetry.progressionSetups.push(setup);
        const advanced = await waitSnake(
          snap => snakeSteps(snap) > snakeSteps(setup) || ['won', 'complete', 'completed'].includes(snakeState(snap))
        );
        report.telemetry.transitions.push(advanced);
        latest = advanced;
      }
    }
  } else if (profileName === 'endless_runner') {
    await sleep(200);
    report.canvasStart = sampleCanvas();
    for (const code of ['Space', 'ArrowUp']) key('keydown', code);
    await sleep(90);
    report.telemetry.duringJump = snapshot();
    for (const code of ['Space', 'ArrowUp']) key('keyup', code);
    await sleep(160);

    for (const code of ['ArrowDown', 'KeyS']) key('keydown', code);
    await sleep(150);
    report.telemetry.afterRight = snapshot();
    for (const code of ['ArrowDown', 'KeyS']) key('keyup', code);
    await sleep(100);

    if (isV2 && api && typeof api.action === 'function') {
      try { await Promise.resolve(api.action('damage')); } catch (e) { probe.errors.push('beast damage failed: ' + e); }
      await sleep(100);
      report.telemetry.afterDamage = snapshot();

      try { await Promise.resolve(api.action('fail')); } catch (e) { probe.errors.push('beast fail failed: ' + e); }
      await sleep(100);
      report.telemetry.transitions = [snapshot()];
    }
  } else if (profileName === 'shooter') {
    for (const code of ['ArrowRight', 'KeyD']) key('keydown', code);
    await sleep(280);
    report.canvasAfterRight = sampleCanvas();
    report.telemetry.afterRight = snapshot();
    for (const code of ['ArrowRight', 'KeyD']) key('keyup', code);

    report.telemetry.beforeFire = snapshot();
    key('keydown', 'Space');
    await sleep(90);
    report.telemetry.afterFire = snapshot();
    key('keyup', 'Space');
    await sleep(80);

    if (isV2 && api && typeof api.action === 'function') {
      report.actionResults = {};
      try {
        const result = await Promise.resolve(api.action('damage'));
        report.actionResults.damage = result === true;
      } catch (e) {
        report.actionResults.damage = false;
        probe.errors.push('beast damage failed: ' + e);
      }
      await sleep(100);
      report.telemetry.afterDamage = snapshot();

      try {
        const result = await Promise.resolve(api.action('advance'));
        report.actionResults.advance = result === true;
      } catch (e) {
        report.actionResults.advance = false;
        probe.errors.push('beast advance failed: ' + e);
      }
      await sleep(100);
      report.telemetry.transitions = [snapshot()];
    }
  } else if (profileName === 'breakout_pinball') {
    key('keydown', 'Space');
    await sleep(40);
    key('keyup', 'Space');
    await sleep(80);
    key('keydown', 'ArrowRight');
    await sleep(220);
    report.canvasAfterRight = sampleCanvas();
    report.telemetry.afterRight = snapshot();
    key('keyup', 'ArrowRight');

    if (isV2 && api && typeof api.action === 'function') {
      try { await Promise.resolve(api.action('hit_target')); } catch (e) { probe.errors.push('beast hit_target failed: ' + e); }
      await sleep(100);
      report.telemetry.afterHit = snapshot();

      try { await Promise.resolve(api.action('drain')); } catch (e) { probe.errors.push('beast drain failed: ' + e); }
      await sleep(100);
      report.telemetry.afterDamage = snapshot();

      try { await Promise.resolve(api.action('advance')); } catch (e) { probe.errors.push('beast advance failed: ' + e); }
      await sleep(100);
      report.telemetry.transitions = [snapshot()];
    }
  } else if (profileName === 'puzzle_grid') {
    key('keydown', 'ArrowRight');
    await sleep(120);
    key('keyup', 'ArrowRight');
    report.canvasAfterRight = sampleCanvas();
    report.telemetry.afterRight = snapshot();

    if (isV2 && api && typeof api.action === 'function') {
      try { await Promise.resolve(api.action('move')); } catch (e) { probe.errors.push('beast move failed: ' + e); }
      await sleep(100);
      report.telemetry.afterMove = snapshot();

      try { await Promise.resolve(api.action('push')); } catch (e) { probe.errors.push('beast push failed: ' + e); }
      await sleep(100);
      report.telemetry.afterPush = snapshot();

      try { await Promise.resolve(api.action('advance')); } catch (e) { probe.errors.push('beast advance failed: ' + e); }
      await sleep(100);
      report.telemetry.transitions = [snapshot()];
    }
  } else if (profileName === 'match3') {
    const board = document.querySelector('canvas');
    const metricsOf = snap => snap && snap.metrics ? snap.metrics : {};
    const clickCell = (cell, geometry) => {
      if (!board || !cell || !geometry) return;
      const rect = board.getBoundingClientRect();
      const x = Number(geometry.origin_x) + Number(cell.col) * Number(geometry.stride_x);
      const y = Number(geometry.origin_y) + Number(cell.row) * Number(geometry.stride_y);
      board.dispatchEvent(new MouseEvent('click', {
        clientX: rect.left + (x / board.width) * rect.width,
        clientY: rect.top + (y / board.height) * rect.height,
        bubbles: true
      }));
    };
    const waitStableMove = async minMoves => {
      let latest = snapshot();
      for (let i = 0; i < 36; i++) {
        if (latest && latest.board && latest.board.stable === true && Number(metricsOf(latest).moves) >= minMoves) return latest;
        await sleep(50);
        latest = snapshot();
      }
      return latest;
    };
    const initialMatch = report.telemetry.initial || {};
    const hint = initialMatch.probe && initialMatch.probe.legal_swap;
    const geometry = initialMatch.board && initialMatch.board.geometry;
    const initialMoves = Number(metricsOf(initialMatch).moves || 0);
    clickCell(hint && hint.from, geometry);
    await sleep(80);
    report.telemetry.afterSelect = snapshot();
    clickCell(hint && hint.to, geometry);
    report.telemetry.afterPointer = await waitStableMove(initialMoves + 1);
    report.canvasAfterRight = sampleCanvas();

    if (isV2 && api && typeof api.action === 'function') {
      const pointerMoves = Number(metricsOf(report.telemetry.afterPointer).moves || initialMoves);
      try { await Promise.resolve(api.action('legal_swap')); } catch (e) { probe.errors.push('beast legal_swap failed: ' + e); }
      report.telemetry.afterSwap = await waitStableMove(pointerMoves + 1);
      report.telemetry.transitions = [report.telemetry.afterPointer, report.telemetry.afterSwap];
      const initialLevel = Number(metricsOf(initialMatch).level || 0);
      for (let i = 0; i < 12; i++) {
        const latest = report.telemetry.transitions[report.telemetry.transitions.length - 1];
        if (!latest || Number(metricsOf(latest).level || 0) > initialLevel || ['won', 'complete', 'completed'].includes(latest.state)) break;
        const beforeMoves = Number(metricsOf(latest).moves || 0);
        try { await Promise.resolve(api.action('legal_swap')); } catch (e) { probe.errors.push('beast legal_swap progression failed: ' + e); break; }
        report.telemetry.transitions.push(await waitStableMove(beforeMoves + 1));
      }
    }
  } else if (profileName === 'tower_defense') {
    const board = document.querySelector('canvas');
    const metricsOf = snap => snap && snap.metrics ? snap.metrics : {};
    const towerCount = snap => Number(metricsOf(snap).towers || 0);
    const enemyList = snap => snap && Array.isArray(snap.enemies) ? snap.enemies : [];
    const clickBuildCell = (hint, geometry) => {
      if (!board || !hint || !geometry) return;
      const rect = board.getBoundingClientRect();
      const x = Number(geometry.origin_x) + Number(hint.col) * Number(geometry.stride_x) + Number(geometry.cell_w) / 2;
      const y = Number(geometry.origin_y) + Number(hint.row) * Number(geometry.stride_y) + Number(geometry.cell_h) / 2;
      board.dispatchEvent(new MouseEvent('click', {
        clientX: rect.left + (x / board.width) * rect.width,
        clientY: rect.top + (y / board.height) * rect.height,
        bubbles: true
      }));
    };
    const waitFor = async (predicate, attempts = 24, interval = 60) => {
      let latest = snapshot();
      for (let i = 0; i < attempts; i++) {
        if (latest && predicate(latest)) return latest;
        await sleep(interval);
        latest = snapshot();
      }
      return latest;
    };
    const invoke = async (name, resultName = name) => {
      try {
        const result = await Promise.resolve(api.action(name));
        report.actionResults[resultName] = result === true;
      } catch (e) {
        report.actionResults[resultName] = false;
        probe.errors.push('beast ' + name + ' failed: ' + e);
      }
    };

    report.actionResults = {};
    const initialDefense = report.telemetry.initial || {};
    const initialHint = initialDefense.probe && initialDefense.probe.place_tower;
    const initialGeometry = initialDefense.board && initialDefense.board.geometry;
    clickBuildCell(initialHint, initialGeometry);
    report.telemetry.afterPointer = await waitFor(snap => towerCount(snap) === towerCount(initialDefense) + 1);
    report.canvasAfterRight = sampleCanvas();

    if (isV2 && api && typeof api.action === 'function') {
      const pointerDefense = report.telemetry.afterPointer || {};
      await invoke('place_tower');
      report.telemetry.afterPlace = await waitFor(snap => towerCount(snap) === towerCount(pointerDefense) + 1);

      const placementDone = report.telemetry.afterPlace || pointerDefense;
      const spawnedBefore = Number(metricsOf(placementDone).spawned || 0);
      await invoke('start_wave');
      report.telemetry.afterWaveStart = await waitFor(
        snap => Number(metricsOf(snap).spawned || 0) > spawnedBefore && enemyList(snap).length > 0,
        28,
        60
      );

      const waveStarted = report.telemetry.afterWaveStart || placementDone;
      const killsBefore = Number(metricsOf(waveStarted).kills || 0);
      const hitsBefore = Number(metricsOf(waveStarted).hits || 0);
      report.telemetry.afterCombat = await waitFor(
        snap => Number(metricsOf(snap).kills || 0) > killsBefore && Number(metricsOf(snap).hits || 0) > hitsBefore,
        36,
        70
      );

      const completedBefore = Number(metricsOf(placementDone).completed_waves || 0);
      const waveBefore = Number(metricsOf(placementDone).wave || 0);
      report.telemetry.afterWaveProgress = await waitFor(
        snap => Number(metricsOf(snap).completed_waves || 0) > completedBefore &&
          (Number(metricsOf(snap).wave || 0) > waveBefore || ['won', 'complete', 'completed'].includes(snap.state)),
        36,
        70
      );
      report.telemetry.transitions = [report.telemetry.afterWaveProgress];

      const progressed = report.telemetry.afterWaveProgress || report.telemetry.afterCombat || waveStarted;
      if (progressed && !['won', 'complete', 'completed'].includes(progressed.state)) {
        const spawnedAtProgress = Number(metricsOf(progressed).spawned || 0);
        await invoke('start_wave', 'start_wave_for_leak');
        report.telemetry.beforeLeak = await waitFor(
          snap => Number(metricsOf(snap).spawned || 0) > spawnedAtProgress && enemyList(snap).length > 0,
          28,
          60
        );
        const beforeLeak = report.telemetry.beforeLeak || progressed;
        const hpBefore = Number(metricsOf(beforeLeak).base_hp);
        const leaksBefore = Number(metricsOf(beforeLeak).leaks || 0);
        await invoke('leak');
        report.telemetry.afterLeak = await waitFor(
          snap => Number(metricsOf(snap).leaks || 0) > leaksBefore && Number(metricsOf(snap).base_hp) < hpBefore,
          24,
          60
        );
      }
    }
  } else if (profileName === 'rhythm') {
    const metricsOf = snap => snap && snap.metrics ? snap.metrics : {};
    const pendingNote = snap => snap && snap.probe ? snap.probe.next_note : null;
    const noteJudgement = (snap, id) => {
      const chart = snap && Array.isArray(snap.chart) ? snap.chart : [];
      const note = chart.find(item => item && item.id === id);
      return note ? note.judgement : undefined;
    };
    const waitFor = async (predicate, attempts = 40, interval = 30) => {
      let latest = snapshot();
      for (let i = 0; i < attempts; i++) {
        if (latest && predicate(latest)) return latest;
        await sleep(interval);
        latest = snapshot();
      }
      return latest;
    };
    const waitHittable = async () => waitFor(snap => {
      const note = pendingNote(snap);
      const timing = snap && snap.track && snap.track.timing;
      const clock = Number(metricsOf(snap).clock_ms);
      const perfect = Number(timing && timing.perfect_ms);
      return note && Number.isFinite(clock) && Number.isFinite(perfect) &&
        Math.abs(Number(note.time_ms) - clock) <= Math.max(10, perfect * 0.75);
    }, 90, 30);
    const dispatchLane = hint => {
      const raw = String((hint && hint.key) || 'd').toLowerCase();
      const eventKey = /^[a-z]$/.test(raw) ? raw : 'd';
      const eventCode = 'Key' + eventKey.toUpperCase();
      const target = document.querySelector('canvas') || document.body || document;
      if (typeof target.focus === 'function') target.focus();
      target.dispatchEvent(new KeyboardEvent('keydown', {
        code: eventCode, key: eventKey, bubbles: true, cancelable: true
      }));
      target.dispatchEvent(new KeyboardEvent('keyup', {
        code: eventCode, key: eventKey, bubbles: true, cancelable: true
      }));
      return { code: eventCode, key: eventKey, note_id: hint && hint.id, lane: hint && hint.lane };
    };
    const invoke = async name => {
      try {
        const result = await Promise.resolve(api.action(name));
        report.actionResults[name] = result === true;
      } catch (e) {
        report.actionResults[name] = false;
        probe.errors.push('beast ' + name + ' failed: ' + e);
      }
    };

    report.actionResults = {};
    report.telemetry.beforeOrdinary = await waitHittable();
    const ordinaryHint = pendingNote(report.telemetry.beforeOrdinary);
    report.ordinaryLaneInput = dispatchLane(ordinaryHint);
    report.telemetry.afterOrdinary = await waitFor(
      snap => ordinaryHint && ['perfect', 'good'].includes(noteJudgement(snap, ordinaryHint.id)),
      36,
      30
    );
    report.canvasAfterRight = sampleCanvas();

    if (isV2 && api && typeof api.action === 'function') {
      report.telemetry.beforeHit = await waitHittable();
      const beforeHitRemaining = Number(metricsOf(report.telemetry.beforeHit).remaining_notes);
      await invoke('hit_next');
      report.telemetry.afterHit = await waitFor(
        snap => Number(metricsOf(snap).remaining_notes) === beforeHitRemaining - 1,
        36,
        30
      );

      report.telemetry.beforeMiss = snapshot();
      const beforeMissRemaining = Number(metricsOf(report.telemetry.beforeMiss).remaining_notes);
      await invoke('miss_next');
      report.telemetry.afterMiss = await waitFor(
        snap => Number(metricsOf(snap).remaining_notes) === beforeMissRemaining - 1,
        36,
        30
      );

      report.telemetry.beforeFinish = snapshot();
      await invoke('finish');
      report.telemetry.afterFinish = await waitFor(
        snap => snap && (snap.complete === true || ['results', 'complete', 'completed', 'finished'].includes(snap.state)),
        40,
        30
      );
      report.telemetry.transitions = [report.telemetry.afterFinish];

      const restart = findButton(/restart|retry|again/i);
      report.clickedRestart = Boolean(restart);
      if (restart) restart.click();
      report.telemetry.afterRestart = await waitFor(snap => {
        const metrics = metricsOf(snap);
        return snap && snap.state === 'playing' && snap.complete === false &&
          Number(metrics.remaining_notes) === Number(metrics.total_notes) &&
          Number(metrics.clock_ms) <= 250;
      }, 36, 30);
    }
  } else if (profileName === 'frogger') {
    key('keydown', 'ArrowUp');
    await sleep(140);
    key('keyup', 'ArrowUp');
    report.canvasAfterRight = sampleCanvas();
    report.telemetry.afterRight = snapshot();

    if (isV2 && api && typeof api.action === 'function') {
      try { await Promise.resolve(api.action('collide')); } catch (e) { probe.errors.push('beast collide failed: ' + e); }
      await sleep(120);
      report.telemetry.afterDamage = snapshot();

      report.telemetry.transitions = [];
      const initialLevel = Number((report.telemetry.initial && report.telemetry.initial.metrics && report.telemetry.initial.metrics.level) || 0);
      for (let i = 0; i < 6; i++) {
        try { await Promise.resolve(api.action('reach_goal')); } catch (e) { probe.errors.push('beast reach_goal failed: ' + e); break; }
        await sleep(120);
        const latest = snapshot();
        report.telemetry.transitions.push(latest);
        const level = Number((latest && latest.metrics && latest.metrics.level) || 0);
        if (level > initialLevel || (latest && ['won', 'complete', 'completed'].includes(latest.state))) break;
      }
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


def _first_num(*values: float | None) -> float | None:
    """Return the first present number without treating the valid value zero as absent."""
    return next((value for value in values if value is not None), None)


def _terminal(snapshot: Any) -> bool:
    if not isinstance(snapshot, dict):
        return False
    return snapshot.get("complete") is True or bool(
        re.search(r"won|win|victory|complete|finished", str(snapshot.get("state", "")), re.IGNORECASE)
    )


def _snake_coord(value: Any) -> tuple[int, int] | None:
    if not isinstance(value, dict):
        return None
    x, y = value.get("x"), value.get("y")
    if not isinstance(x, int) or isinstance(x, bool) or not isinstance(y, int) or isinstance(y, bool):
        return None
    return x, y


def _snake_snapshot(snapshot: Any, label: str) -> tuple[dict[str, Any] | None, list[str]]:
    reasons: list[str] = []
    if not isinstance(snapshot, dict):
        return None, [f"beast telemetry: {label} snake snapshot missing"]
    board = snapshot.get("board")
    actor = snapshot.get("actor")
    metrics = snapshot.get("metrics")
    if not all(isinstance(value, dict) for value in (board, actor, metrics)):
        return None, [f"beast telemetry: {label} snake board/actor/metrics missing"]

    state = snapshot.get("state")
    complete = snapshot.get("complete")
    score = snapshot.get("score")
    if not isinstance(state, str) or not state.strip():
        reasons.append(f"beast telemetry: {label} snake state missing")
    if not isinstance(complete, bool):
        reasons.append(f"beast telemetry: {label} snake complete flag is not boolean")
    if not isinstance(score, (int, float)) or isinstance(score, bool) or score < 0:
        reasons.append(f"beast telemetry: {label} snake score is not non-negative")

    rows, cols, body = board.get("rows"), board.get("cols"), board.get("body")
    dimensions_ok = (
        isinstance(rows, int) and not isinstance(rows, bool) and rows >= 6
        and isinstance(cols, int) and not isinstance(cols, bool) and cols >= 6
    )
    if not dimensions_ok:
        reasons.append(f"beast telemetry: {label} snake board dimensions invalid")
    body_coords = [_snake_coord(item) for item in body] if isinstance(body, list) else []
    body_ok = isinstance(body, list) and len(body) >= 3 and all(item is not None for item in body_coords)
    if body_ok and dimensions_ok:
        body_ok = (
            len(set(body_coords)) == len(body_coords)
            and all(0 <= x < cols and 0 <= y < rows for x, y in body_coords)
            and all(
                abs(x1 - x0) + abs(y1 - y0) == 1
                for (x0, y0), (x1, y1) in zip(body_coords, body_coords[1:])
            )
        )
    if not body_ok:
        reasons.append(f"beast telemetry: {label} snake body is not a unique contiguous in-bounds chain")

    actor_coord = _snake_coord(actor)
    if actor_coord is None or (body_coords and actor_coord != body_coords[0]):
        reasons.append(f"beast telemetry: {label} snake actor does not match board.body head")

    direction = metrics.get("direction")
    direction_coord = _snake_coord(direction)
    if direction_coord is None or abs(direction_coord[0]) + abs(direction_coord[1]) != 1:
        reasons.append(f"beast telemetry: {label} snake direction is not a cardinal unit vector")

    food_coord = _snake_coord(metrics.get("food"))
    if (
        food_coord is None
        or (dimensions_ok and not (0 <= food_coord[0] < cols and 0 <= food_coord[1] < rows))
        or food_coord in body_coords
    ):
        reasons.append(f"beast telemetry: {label} snake food is not a free in-bounds cell")

    counters: dict[str, int] = {}
    for name in ("length", "steps", "foods_eaten", "deaths", "target_foods"):
        value = metrics.get(name)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            reasons.append(f"beast telemetry: {label} metrics.{name} is not a non-negative integer")
        else:
            counters[name] = value
    if counters.get("length") != len(body_coords):
        reasons.append(f"beast telemetry: {label} metrics.length does not match board.body")
    if counters.get("target_foods", 0) < 2:
        reasons.append(f"beast telemetry: {label} finite target_foods must be at least two")

    state_key = str(state or "").lower()
    won = state_key in {"won", "win", "complete", "completed"}
    lost = state_key in {"lost", "gameover", "game_over"}
    if complete is True and not won:
        reasons.append(f"beast telemetry: {label} complete flag is true outside a win state")
    if won and complete is not True:
        reasons.append(f"beast telemetry: {label} win state does not set complete=true")
    if lost and complete is not False:
        reasons.append(f"beast telemetry: {label} loss state must not report completion")
    if (
        state_key == "playing"
        and "foods_eaten" in counters and "target_foods" in counters
        and counters["foods_eaten"] >= counters["target_foods"]
    ):
        reasons.append(f"beast telemetry: {label} remained playing after reaching target_foods")

    data = {
        "snapshot": snapshot,
        "state": state_key,
        "complete": complete,
        "score": score,
        "rows": rows,
        "cols": cols,
        "body": body_coords,
        "actor": actor_coord,
        "direction": direction_coord,
        "food": food_coord,
        "metrics": counters,
    }
    return (data if not reasons else None), reasons


def _snake_unchanged(before: dict[str, Any], after: dict[str, Any], label: str, names: tuple[str, ...]) -> list[str]:
    reasons: list[str] = []
    for name in names:
        old = before["score"] if name == "score" else before["metrics"].get(name)
        new = after["score"] if name == "score" else after["metrics"].get(name)
        if new != old:
            reasons.append(f"beast telemetry: {label} changed {name} before a real shared tick")
    if after["state"] != before["state"] or after["complete"] != before["complete"]:
        reasons.append(f"beast telemetry: {label} changed terminal state before a real shared tick")
    return reasons


def _snake_move_transition(
    before: dict[str, Any] | None,
    after: dict[str, Any] | None,
    label: str,
    expected_direction: tuple[int, int],
) -> list[str]:
    if before is None or after is None:
        return []
    reasons: list[str] = []
    if after["direction"] != expected_direction:
        reasons.append(f"beast telemetry: {label} did not commit the requested cardinal direction")
    expected_head = (before["actor"][0] + expected_direction[0], before["actor"][1] + expected_direction[1])
    if after["actor"] != expected_head:
        reasons.append(f"beast telemetry: {label} did not move the head exactly one grid cell")
    if after["body"][1:] != before["body"][:-1]:
        reasons.append(f"beast telemetry: {label} did not shift the canonical body through the real movement path")
    if after["metrics"].get("steps") != before["metrics"].get("steps", -1) + 1:
        reasons.append(f"beast telemetry: {label} did not increment steps exactly once")
    for name in ("length", "foods_eaten", "deaths", "target_foods"):
        if after["metrics"].get(name) != before["metrics"].get(name):
            reasons.append(f"beast telemetry: {label} changed unrelated metrics.{name}")
    if after["score"] != before["score"] or after["food"] != before["food"]:
        reasons.append(f"beast telemetry: {label} awarded food/score during an ordinary empty-cell move")
    if after["state"] != "playing" or after["complete"] is not False:
        reasons.append(f"beast telemetry: {label} did not remain in the playing state")
    return reasons


def _snake_feed_setup_transition(before: dict[str, Any] | None, setup: dict[str, Any] | None, label: str) -> list[str]:
    if before is None or setup is None:
        return []
    reasons = _snake_unchanged(
        before, setup, label, ("score", "length", "steps", "foods_eaten", "deaths", "target_foods")
    )
    if setup["body"] != before["body"] or setup["actor"] != before["actor"] or setup["direction"] != before["direction"]:
        reasons.append(f"beast telemetry: {label} rewrote the snake instead of only staging food")
    expected_food = (setup["actor"][0] + setup["direction"][0], setup["actor"][1] + setup["direction"][1])
    if setup["food"] != expected_food:
        reasons.append(f"beast telemetry: {label} did not stage food on the next real head cell")
    return reasons


def _snake_feed_transition(setup: dict[str, Any] | None, after: dict[str, Any] | None, label: str) -> list[str]:
    if setup is None or after is None:
        return []
    reasons: list[str] = []
    if after["actor"] != setup["food"]:
        reasons.append(f"beast telemetry: {label} did not consume staged food through head movement")
    if after["body"][1:] != setup["body"]:
        reasons.append(f"beast telemetry: {label} did not preserve the old tail during real growth")
    if after["metrics"].get("steps") != setup["metrics"].get("steps", -1) + 1:
        reasons.append(f"beast telemetry: {label} did not increment steps exactly once")
    if after["metrics"].get("length") != setup["metrics"].get("length", -1) + 1:
        reasons.append(f"beast telemetry: {label} did not grow length exactly once")
    if after["metrics"].get("foods_eaten") != setup["metrics"].get("foods_eaten", -1) + 1:
        reasons.append(f"beast telemetry: {label} did not increment foods_eaten exactly once")
    if after["score"] <= setup["score"]:
        reasons.append(f"beast telemetry: {label} did not earn real score")
    for name in ("deaths", "target_foods"):
        if after["metrics"].get(name) != setup["metrics"].get(name):
            reasons.append(f"beast telemetry: {label} changed unrelated metrics.{name}")
    reached_target = after["metrics"].get("foods_eaten", 0) >= after["metrics"].get("target_foods", 1)
    if reached_target and not (after["complete"] is True and after["state"] in {"won", "win", "complete", "completed"}):
        reasons.append(f"beast telemetry: {label} reached target_foods without a finite win")
    if not reached_target and (after["complete"] is not False or after["state"] != "playing"):
        reasons.append(f"beast telemetry: {label} became terminal before the finite target")
    return reasons


def _fnv1a_json(value: Any) -> str:
    canonical = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    value = 2166136261
    for char in canonical:
        value ^= ord(char)
        value = (value * 16777619) & 0xFFFFFFFF
    return f"{value:08x}"


def _fnv1a_cells(cells: list[list[str]]) -> str:
    return _fnv1a_json(cells)


def _rhythm_chart_hash(chart: list[dict[str, Any]]) -> str:
    immutable = [[note["id"], note["lane"], note["time_ms"]] for note in chart]
    return _fnv1a_json(immutable)


def _rhythm_snapshot(snapshot: Any, label: str) -> tuple[dict[str, Any] | None, list[str]]:
    reasons: list[str] = []
    if not isinstance(snapshot, dict):
        return None, [f"beast telemetry: {label} rhythm snapshot missing"]
    track = snapshot.get("track")
    chart = snapshot.get("chart")
    probe = snapshot.get("probe")
    metrics = snapshot.get("metrics")
    if not all(isinstance(value, dict) for value in (track, probe, metrics)) or not isinstance(chart, list):
        return None, [f"beast telemetry: {label} rhythm track/chart/probe/metrics missing"]

    track_id = track.get("id")
    duration = track.get("duration_ms")
    lanes = track.get("lanes")
    lane_keys = track.get("lane_keys")
    timing = track.get("timing")
    if not isinstance(track_id, str) or not track_id.strip():
        reasons.append(f"beast telemetry: {label} track.id is invalid")
    if not isinstance(duration, int) or isinstance(duration, bool) or duration <= 0:
        reasons.append(f"beast telemetry: {label} track.duration_ms is invalid")
    if not isinstance(lanes, int) or isinstance(lanes, bool) or not 2 <= lanes <= 8:
        reasons.append(f"beast telemetry: {label} track.lanes is invalid")
    keys_ok = (
        isinstance(lane_keys, list) and isinstance(lanes, int) and len(lane_keys) == lanes
        and all(isinstance(key, str) and bool(re.fullmatch(r"[a-z]", key)) for key in lane_keys)
        and len(set(lane_keys)) == len(lane_keys)
    )
    if not keys_ok:
        reasons.append(f"beast telemetry: {label} lane_keys must be unique lowercase lane keys")
    perfect = timing.get("perfect_ms") if isinstance(timing, dict) else None
    good = timing.get("good_ms") if isinstance(timing, dict) else None
    timing_ok = (
        isinstance(perfect, (int, float)) and not isinstance(perfect, bool)
        and isinstance(good, (int, float)) and not isinstance(good, bool)
        and 0 < float(perfect) < float(good) <= 250
    )
    if not timing_ok:
        reasons.append(f"beast telemetry: {label} rhythm timing windows are invalid")

    if len(chart) < 4:
        reasons.append(f"beast telemetry: {label} finite rhythm chart has fewer than four notes")
    valid_chart: list[dict[str, Any]] = []
    note_ids: set[str] = set()
    previous_time = -1
    judgements = {"perfect": 0, "good": 0, "miss": 0, "pending": 0}
    for note in chart:
        valid = (
            isinstance(note, dict)
            and isinstance(note.get("id"), str) and bool(re.fullmatch(r"[A-Za-z0-9_.:-]+", note.get("id", "")))
            and note.get("id") not in note_ids
            and isinstance(note.get("lane"), int) and not isinstance(note.get("lane"), bool)
            and isinstance(note.get("time_ms"), int) and not isinstance(note.get("time_ms"), bool)
            and note.get("judgement") in {None, "perfect", "good", "miss"}
        )
        if valid and isinstance(lanes, int) and isinstance(duration, int):
            valid = (
                0 <= note["lane"] < lanes
                and previous_time <= note["time_ms"] <= duration
            )
        if not valid:
            reasons.append(f"beast telemetry: {label} chart contains an invalid, duplicate, or unsorted note")
            continue
        note_ids.add(note["id"])
        previous_time = note["time_ms"]
        judgement = note["judgement"] if note["judgement"] is not None else "pending"
        judgements[judgement] += 1
        valid_chart.append(note)
    if len(valid_chart) != len(chart):
        return None, reasons

    expected_hash = _rhythm_chart_hash(valid_chart)
    if str(metrics.get("chart_hash") or "").lower() != expected_hash:
        reasons.append(f"beast telemetry: {label} metrics.chart_hash is not canonical for immutable chart notes")

    counter_names = (
        "combo", "max_combo", "hits", "perfect", "good", "misses",
        "judged_notes", "remaining_notes", "total_notes",
    )
    counters: dict[str, int] = {}
    for name in counter_names:
        value = metrics.get(name)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            reasons.append(f"beast telemetry: {label} metrics.{name} is not a non-negative integer")
        else:
            counters[name] = value
    clock = metrics.get("clock_ms")
    if not isinstance(clock, (int, float)) or isinstance(clock, bool) or float(clock) < 0:
        reasons.append(f"beast telemetry: {label} metrics.clock_ms is invalid")
    elif isinstance(duration, int) and timing_ok and float(clock) > duration + float(good):
        reasons.append(f"beast telemetry: {label} metrics.clock_ms exceeds the finite track")
    accuracy = metrics.get("accuracy")
    if not isinstance(accuracy, (int, float)) or isinstance(accuracy, bool) or not 0 <= float(accuracy) <= 1:
        reasons.append(f"beast telemetry: {label} metrics.accuracy is outside 0..1")
    score = snapshot.get("score")
    if not isinstance(score, (int, float)) or isinstance(score, bool) or score < 0:
        reasons.append(f"beast telemetry: {label} score is invalid")

    expected_counts = {
        "perfect": judgements["perfect"],
        "good": judgements["good"],
        "misses": judgements["miss"],
        "hits": judgements["perfect"] + judgements["good"],
        "judged_notes": judgements["perfect"] + judgements["good"] + judgements["miss"],
        "remaining_notes": judgements["pending"],
        "total_notes": len(valid_chart),
    }
    for name, expected in expected_counts.items():
        if counters.get(name) != expected:
            reasons.append(f"beast telemetry: {label} metrics.{name} does not match chart judgements")
    if "combo" in counters and "max_combo" in counters and "hits" in counters:
        if counters["combo"] > counters["max_combo"] or counters["max_combo"] > counters["hits"]:
            reasons.append(f"beast telemetry: {label} combo accounting is impossible")

    pending = [note for note in valid_chart if note["judgement"] is None]
    next_note = probe.get("next_note")
    if pending:
        expected = pending[0]
        expected_key = lane_keys[expected["lane"]] if keys_ok else None
        hint_ok = (
            isinstance(next_note, dict)
            and next_note.get("id") == expected["id"]
            and next_note.get("lane") == expected["lane"]
            and next_note.get("time_ms") == expected["time_ms"]
            and next_note.get("key") == expected_key
        )
        if not hint_ok:
            reasons.append(f"beast telemetry: {label} next_note does not identify the earliest real pending note")
    elif next_note not in (None, {}):
        reasons.append(f"beast telemetry: {label} completed chart still advertises a pending note")

    state = snapshot.get("state")
    if not isinstance(state, str) or not state:
        reasons.append(f"beast telemetry: {label} rhythm state is invalid")
    if not isinstance(snapshot.get("complete"), bool):
        reasons.append(f"beast telemetry: {label} complete flag is not boolean")

    identities = [(note["id"], note["lane"], note["time_ms"]) for note in valid_chart]
    statuses = {note["id"]: note["judgement"] for note in valid_chart}
    return {
        "snapshot": snapshot,
        "track": (track_id, duration, lanes, tuple(lane_keys) if isinstance(lane_keys, list) else (), perfect, good),
        "chart": valid_chart,
        "identities": identities,
        "statuses": statuses,
        "hash": expected_hash,
        "next_note": next_note,
        "metrics": metrics,
        "score": score,
    }, reasons


def _rhythm_single_transition(
    before: dict[str, Any] | None,
    after: dict[str, Any] | None,
    label: str,
    *,
    expected: str,
) -> list[str]:
    if before is None or after is None:
        return []
    reasons: list[str] = []
    if before["track"] != after["track"] or before["identities"] != after["identities"] or before["hash"] != after["hash"]:
        reasons.append(f"beast telemetry: {label} rewrote the immutable rhythm chart")
        return reasons
    changes = [
        (note_id, judgement, after["statuses"].get(note_id))
        for note_id, judgement in before["statuses"].items()
        if after["statuses"].get(note_id) != judgement
    ]
    allowed = {"perfect", "good"} if expected == "hit" else {"miss"}
    if len(changes) != 1 or changes[0][1] is not None or changes[0][2] not in allowed:
        reasons.append(f"beast telemetry: {label} did not judge exactly one real pending note as {expected}")
    elif not isinstance(before["next_note"], dict) or changes[0][0] != before["next_note"].get("id"):
        reasons.append(f"beast telemetry: {label} judged a note other than the declared next_note")
    if expected == "hit" and isinstance(before["next_note"], dict):
        clock = _num(before["metrics"], "clock_ms")
        note_time = before["next_note"].get("time_ms")
        good_window = before["track"][5]
        if (
            clock is None
            or not isinstance(note_time, (int, float)) or isinstance(note_time, bool)
            or not isinstance(good_window, (int, float)) or isinstance(good_window, bool)
            or abs(clock - float(note_time)) > float(good_window)
        ):
            reasons.append(f"beast telemetry: {label} judged a note outside the declared timing window")

    old, new = before["metrics"], after["metrics"]
    if _num(new, "remaining_notes") != (_num(old, "remaining_notes") or 0) - 1:
        reasons.append(f"beast telemetry: {label} did not decrement remaining_notes exactly once")
    if _num(new, "judged_notes") != (_num(old, "judged_notes") or 0) + 1:
        reasons.append(f"beast telemetry: {label} did not increment judged_notes exactly once")
    if _num(new, "total_notes") != _num(old, "total_notes"):
        reasons.append(f"beast telemetry: {label} changed total_notes")
    if (_num(new, "clock_ms") or 0) < (_num(old, "clock_ms") or 0):
        reasons.append(f"beast telemetry: {label} moved the song clock backwards")

    if expected == "hit":
        if _num(new, "hits") != (_num(old, "hits") or 0) + 1:
            reasons.append(f"beast telemetry: {label} did not increment hits exactly once")
        if _num(new, "misses") != _num(old, "misses"):
            reasons.append(f"beast telemetry: {label} also changed misses")
        positive_delta = sum(
            (_num(new, name) or 0) - (_num(old, name) or 0)
            for name in ("perfect", "good")
        )
        if positive_delta != 1:
            reasons.append(f"beast telemetry: {label} did not record exactly one positive judgement")
        if not isinstance(before["score"], (int, float)) or not isinstance(after["score"], (int, float)) or after["score"] <= before["score"]:
            reasons.append(f"beast telemetry: {label} did not increase real score")
        if _num(new, "combo") != (_num(old, "combo") or 0) + 1:
            reasons.append(f"beast telemetry: {label} did not advance combo exactly once")
        if (_num(new, "max_combo") or 0) < (_num(old, "max_combo") or 0):
            reasons.append(f"beast telemetry: {label} reduced max_combo")
    else:
        if _num(new, "misses") != (_num(old, "misses") or 0) + 1:
            reasons.append(f"beast telemetry: {label} did not increment misses exactly once")
        for name in ("hits", "perfect", "good", "max_combo"):
            if _num(new, name) != _num(old, name):
                reasons.append(f"beast telemetry: {label} changed unrelated metrics.{name}")
        if after["score"] != before["score"]:
            reasons.append(f"beast telemetry: {label} awarded or removed score for a miss")
        if _num(new, "combo") != 0:
            reasons.append(f"beast telemetry: {label} did not reset combo")
    return reasons


def _match3_has_match(cells: list[list[str]]) -> bool:
    rows = len(cells)
    cols = len(cells[0]) if rows else 0
    for row in range(rows):
        for col in range(cols - 2):
            if cells[row][col] == cells[row][col + 1] == cells[row][col + 2]:
                return True
    for col in range(cols):
        for row in range(rows - 2):
            if cells[row][col] == cells[row + 1][col] == cells[row + 2][col]:
                return True
    return False


def _match3_snapshot(snapshot: Any, label: str) -> tuple[dict[str, Any] | None, list[str]]:
    reasons: list[str] = []
    if not isinstance(snapshot, dict):
        return None, [f"beast telemetry: {label} match3 snapshot missing"]
    board = snapshot.get("board")
    metrics = snapshot.get("metrics")
    probe = snapshot.get("probe")
    if not isinstance(board, dict) or not isinstance(metrics, dict) or not isinstance(probe, dict):
        return None, [f"beast telemetry: {label} match3 board/metrics/probe missing"]
    rows, cols, cells = board.get("rows"), board.get("cols"), board.get("cells")
    if not isinstance(rows, int) or isinstance(rows, bool) or rows < 3 or not isinstance(cols, int) or isinstance(cols, bool) or cols < 3:
        reasons.append(f"beast telemetry: {label} board dimensions invalid")
    valid_cells = (
        isinstance(cells, list) and isinstance(rows, int) and len(cells) == rows
        and all(isinstance(row, list) and isinstance(cols, int) and len(row) == cols for row in cells)
        and all(isinstance(cell, str) and bool(cell) for row in cells for cell in row)
    )
    if not valid_cells:
        reasons.append(f"beast telemetry: {label} board cells are not a full stable identifier grid")
        return None, reasons
    if board.get("stable") is not True:
        reasons.append(f"beast telemetry: {label} board is not stable")
    if _match3_has_match(cells):
        reasons.append(f"beast telemetry: {label} stable board still contains unresolved matches")
    expected_hash = _fnv1a_cells(cells)
    if str(metrics.get("board_hash") or "").lower() != expected_hash:
        reasons.append(f"beast telemetry: {label} metrics.board_hash is not canonical for board.cells")
    geometry = board.get("geometry")
    geometry_keys = ("origin_x", "origin_y", "stride_x", "stride_y", "cell_w", "cell_h")
    if not isinstance(geometry, dict) or any(not isinstance(geometry.get(key), (int, float)) for key in geometry_keys):
        reasons.append(f"beast telemetry: {label} board geometry missing")
    hint = probe.get("legal_swap")
    source = hint.get("from") if isinstance(hint, dict) else None
    target = hint.get("to") if isinstance(hint, dict) else None
    coords_ok = all(
        isinstance(point, dict)
        and isinstance(point.get("row"), int) and not isinstance(point.get("row"), bool)
        and isinstance(point.get("col"), int) and not isinstance(point.get("col"), bool)
        for point in (source, target)
    )
    if coords_ok:
        sr, sc, tr, tc = source["row"], source["col"], target["row"], target["col"]
        coords_ok = (
            0 <= sr < rows and 0 <= tr < rows and 0 <= sc < cols and 0 <= tc < cols
            and abs(sr - tr) + abs(sc - tc) == 1
        )
    if not coords_ok:
        reasons.append(f"beast telemetry: {label} legal_swap hint is not adjacent and in bounds")
    else:
        swapped = [row[:] for row in cells]
        swapped[sr][sc], swapped[tr][tc] = swapped[tr][tc], swapped[sr][sc]
        if not _match3_has_match(swapped):
            reasons.append(f"beast telemetry: {label} legal_swap hint does not create a real match")
    return {"cells": cells, "hash": expected_hash, "metrics": metrics, "score": snapshot.get("score")}, reasons


def _tower_defense_snapshot(snapshot: Any, label: str) -> tuple[dict[str, Any] | None, list[str]]:
    reasons: list[str] = []
    if not isinstance(snapshot, dict):
        return None, [f"beast telemetry: {label} tower_defense snapshot missing"]
    board = snapshot.get("board")
    probe = snapshot.get("probe")
    metrics = snapshot.get("metrics")
    towers = snapshot.get("towers")
    enemies = snapshot.get("enemies")
    if not all(isinstance(value, dict) for value in (board, probe, metrics)):
        return None, [f"beast telemetry: {label} tower_defense board/probe/metrics missing"]
    if not isinstance(towers, list) or not isinstance(enemies, list):
        return None, [f"beast telemetry: {label} tower/enemy rosters missing"]

    rows, cols, cells = board.get("rows"), board.get("cols"), board.get("cells")
    dimensions_ok = (
        isinstance(rows, int) and not isinstance(rows, bool) and rows >= 3
        and isinstance(cols, int) and not isinstance(cols, bool) and cols >= 3
    )
    if not dimensions_ok:
        reasons.append(f"beast telemetry: {label} tower board dimensions invalid")
    valid_cells = (
        dimensions_ok and isinstance(cells, list) and len(cells) == rows
        and all(isinstance(row, list) and len(row) == cols for row in cells)
        and all(isinstance(cell, str) and bool(cell.strip()) for row in cells for cell in row)
    )
    if not valid_cells:
        reasons.append(f"beast telemetry: {label} tower board is not a full identifier grid")
        return None, reasons
    expected_hash = _fnv1a_cells(cells)
    if str(metrics.get("board_hash") or "").lower() != expected_hash:
        reasons.append(f"beast telemetry: {label} metrics.board_hash is not canonical for board.cells")

    geometry = board.get("geometry")
    geometry_keys = ("origin_x", "origin_y", "stride_x", "stride_y", "cell_w", "cell_h")
    geometry_ok = isinstance(geometry, dict) and all(
        isinstance(geometry.get(key), (int, float)) and not isinstance(geometry.get(key), bool)
        for key in geometry_keys
    )
    if geometry_ok:
        geometry_ok = all(float(geometry[key]) > 0 for key in ("stride_x", "stride_y", "cell_w", "cell_h"))
    if not geometry_ok:
        reasons.append(f"beast telemetry: {label} tower board geometry invalid")

    hint = probe.get("place_tower")
    hint_ok = (
        isinstance(hint, dict)
        and isinstance(hint.get("row"), int) and not isinstance(hint.get("row"), bool)
        and isinstance(hint.get("col"), int) and not isinstance(hint.get("col"), bool)
        and isinstance(hint.get("type"), str) and bool(hint.get("type").strip())
        and isinstance(hint.get("cost"), (int, float)) and not isinstance(hint.get("cost"), bool)
        and float(hint.get("cost")) > 0
    )
    if hint_ok:
        hint_ok = 0 <= hint["row"] < rows and 0 <= hint["col"] < cols and cells[hint["row"]][hint["col"]] == "build"
    if not hint_ok:
        reasons.append(f"beast telemetry: {label} place_tower hint is not a legal in-bounds build cell")

    tower_ids: set[str] = set()
    tower_positions: set[tuple[int, int]] = set()
    valid_towers: list[dict[str, Any]] = []
    for tower in towers:
        valid = (
            isinstance(tower, dict)
            and isinstance(tower.get("id"), str) and bool(tower.get("id").strip())
            and isinstance(tower.get("row"), int) and not isinstance(tower.get("row"), bool)
            and isinstance(tower.get("col"), int) and not isinstance(tower.get("col"), bool)
            and isinstance(tower.get("type"), str) and bool(tower.get("type").strip())
            and isinstance(tower.get("cost"), (int, float)) and not isinstance(tower.get("cost"), bool)
            and float(tower.get("cost")) > 0
        )
        if valid:
            row, col = tower["row"], tower["col"]
            expected_cell = f"tower:{tower['type']}"
            valid = (
                0 <= row < rows and 0 <= col < cols
                and cells[row][col] == expected_cell
                and tower["id"] not in tower_ids
                and (row, col) not in tower_positions
            )
        if not valid:
            reasons.append(f"beast telemetry: {label} tower roster contains an invalid or duplicate entity")
            continue
        tower_ids.add(tower["id"])
        tower_positions.add((row, col))
        valid_towers.append(tower)
    tower_cells = sum(1 for row in cells for cell in row if cell.startswith("tower:"))
    if tower_cells != len(towers):
        reasons.append(f"beast telemetry: {label} canonical board tower cells do not match tower roster")

    enemy_ids: set[str] = set()
    valid_enemies: list[dict[str, Any]] = []
    for enemy in enemies:
        valid = (
            isinstance(enemy, dict)
            and isinstance(enemy.get("id"), str) and bool(enemy.get("id").strip())
            and isinstance(enemy.get("hp"), (int, float)) and not isinstance(enemy.get("hp"), bool)
            and isinstance(enemy.get("max_hp"), (int, float)) and not isinstance(enemy.get("max_hp"), bool)
            and isinstance(enemy.get("progress"), (int, float)) and not isinstance(enemy.get("progress"), bool)
        )
        if valid:
            valid = (
                0 < float(enemy["hp"]) <= float(enemy["max_hp"])
                and float(enemy["max_hp"]) > 0
                and 0 <= float(enemy["progress"]) <= 1
                and enemy["id"] not in enemy_ids
            )
        if not valid:
            reasons.append(f"beast telemetry: {label} enemy roster contains an invalid or duplicate live entity")
            continue
        enemy_ids.add(enemy["id"])
        valid_enemies.append(enemy)

    counter_names = (
        "towers", "enemies", "wave", "total_waves", "completed_waves",
        "spawned", "kills", "leaks", "shots", "hits", "spent", "earned",
    )
    numeric_names = counter_names + ("currency", "base_hp")
    numbers: dict[str, float] = {}
    for name in numeric_names:
        value = metrics.get(name)
        if not isinstance(value, (int, float)) or isinstance(value, bool) or float(value) < 0:
            reasons.append(f"beast telemetry: {label} metrics.{name} is not a non-negative number")
        else:
            numbers[name] = float(value)
    for name in counter_names:
        if name in numbers and not numbers[name].is_integer():
            reasons.append(f"beast telemetry: {label} metrics.{name} is not an integer counter")
    if numbers.get("towers") != float(len(towers)):
        reasons.append(f"beast telemetry: {label} metrics.towers does not match tower roster")
    if numbers.get("enemies") != float(len(enemies)):
        reasons.append(f"beast telemetry: {label} metrics.enemies does not match live enemy roster")
    if all(name in numbers for name in ("spawned", "kills", "leaks")):
        if numbers["spawned"] != float(len(enemies)) + numbers["kills"] + numbers["leaks"]:
            reasons.append(f"beast telemetry: {label} spawned accounting does not equal live enemies + kills + leaks")
    if all(name in numbers for name in ("shots", "hits", "kills")):
        if numbers["hits"] > numbers["shots"] or numbers["kills"] > numbers["hits"]:
            reasons.append(f"beast telemetry: {label} shot/hit/kill accounting is impossible")
    if "spent" in numbers and numbers["spent"] < sum(float(tower["cost"]) for tower in valid_towers):
        reasons.append(f"beast telemetry: {label} metrics.spent is below the installed tower cost")
    if all(name in numbers for name in ("wave", "total_waves", "completed_waves")):
        if numbers["total_waves"] < 2 or not 1 <= numbers["wave"] <= numbers["total_waves"]:
            reasons.append(f"beast telemetry: {label} finite wave range is invalid")
        if numbers["completed_waves"] > numbers["total_waves"]:
            reasons.append(f"beast telemetry: {label} completed_waves exceeds total_waves")

    return {
        "snapshot": snapshot,
        "cells": cells,
        "hash": expected_hash,
        "hint": hint if hint_ok else None,
        "towers": valid_towers,
        "tower_ids": tower_ids,
        "enemies": valid_enemies,
        "enemy_ids": enemy_ids,
        "metrics": metrics,
    }, reasons


def _tower_placement_transition(
    before: dict[str, Any] | None,
    after: dict[str, Any] | None,
    label: str,
) -> list[str]:
    if before is None or after is None:
        return []
    reasons: list[str] = []
    hint = before.get("hint")
    if not isinstance(hint, dict):
        return [f"beast telemetry: {label} had no legal placement hint"]
    row, col = hint["row"], hint["col"]
    changed = [
        (r, c)
        for r, (old_row, new_row) in enumerate(zip(before["cells"], after["cells"]))
        for c, (old, new) in enumerate(zip(old_row, new_row))
        if old != new
    ]
    if changed != [(row, col)] or after["cells"][row][col] != f"tower:{hint['type']}":
        reasons.append(f"beast telemetry: {label} did not change exactly the hinted build cell into the declared tower")
    added = after["tower_ids"] - before["tower_ids"]
    if len(added) != 1:
        reasons.append(f"beast telemetry: {label} did not add exactly one stable-ID tower")
    else:
        tower = next((item for item in after["towers"] if item["id"] in added), None)
        if not tower or (tower["row"], tower["col"], tower["type"], float(tower["cost"])) != (
            row, col, hint["type"], float(hint["cost"])
        ):
            reasons.append(f"beast telemetry: {label} tower roster does not match the hinted placement")
    before_metrics, after_metrics = before["metrics"], after["metrics"]
    cost = float(hint["cost"])
    if _num(after_metrics, "towers") != (_num(before_metrics, "towers") or 0) + 1:
        reasons.append(f"beast telemetry: {label} did not increment metrics.towers exactly once")
    if _num(after_metrics, "spent") != (_num(before_metrics, "spent") or 0) + cost:
        reasons.append(f"beast telemetry: {label} did not add the declared cost to metrics.spent")
    if _num(after_metrics, "currency") != (_num(before_metrics, "currency") or 0) - cost:
        reasons.append(f"beast telemetry: {label} did not spend the declared currency cost")
    for name in ("earned", "spawned", "kills", "leaks", "shots", "hits", "base_hp", "wave", "completed_waves"):
        if _num(after_metrics, name) != _num(before_metrics, name):
            reasons.append(f"beast telemetry: {label} changed unrelated metrics.{name}")
    return reasons


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
    requested_profile = expected_profile
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
        # Keep collecting universal evidence such as audio instead of masking
        # independent failures behind the missing adapter.
        expected_profile = ""

    telemetry = report.get("telemetry") or {}
    initial = telemetry.get("initial")
    moved = telemetry.get("afterRight")
    damaged = telemetry.get("afterDamage")
    transitions = [item for item in (telemetry.get("transitions") or []) if isinstance(item, dict)]

    if expected_profile == "platformer":
        x0 = _first_num(_num(initial, "actor", "x"), _num(initial, "player", "x"))
        x1 = _first_num(_num(moved, "actor", "x"), _num(moved, "player", "x"))
        if x0 is None or x1 is None or x1 <= x0 + 0.1:
            reasons.append("beast telemetry: holding Right did not advance actor.x/player.x")
        lives0 = _first_num(_num(initial, "metrics", "lives"), _num(initial, "lives"))
        lives1 = _first_num(_num(damaged, "metrics", "lives"), _num(damaged, "lives"))
        if lives0 is None or lives1 is None or lives1 >= lives0:
            reasons.append("beast telemetry: damage action did not reduce lives")
        if not transitions:
            reasons.append("beast telemetry: advance action produced no progression snapshots")
    elif expected_profile == "snake":
        snapshots = {
            "initial": initial,
            "ordinary Down turn": telemetry.get("afterDown"),
            "ordinary Right turn": telemetry.get("afterRight"),
            "feed setup": telemetry.get("afterFeedSetup"),
            "feed tick": telemetry.get("afterFeed"),
            "collision setup": telemetry.get("afterCollisionSetup"),
            "collision tick": telemetry.get("afterDamage"),
            "restart": telemetry.get("afterRestart"),
        }
        snake: dict[str, dict[str, Any] | None] = {}
        for label, item in snapshots.items():
            data, snapshot_reasons = _snake_snapshot(item, label)
            snake[label] = data
            reasons.extend(snapshot_reasons)

        actions = report.get("actionResults") or {}
        for action in ("feed", "collide"):
            if actions.get(action) is not True:
                reasons.append(f"beast telemetry: {action} action did not report a successful setup")
        if report.get("clickedRestart") is not True:
            reasons.append("beast telemetry: snake loss did not expose a visible Restart/Retry action")

        initial_data = snake["initial"]
        down_data = snake["ordinary Down turn"]
        right_data = snake["ordinary Right turn"]
        feed_setup = snake["feed setup"]
        fed_data = snake["feed tick"]
        collision_setup = snake["collision setup"]
        collision_data = snake["collision tick"]
        restart_data = snake["restart"]

        if initial_data is not None and (initial_data["state"] != "playing" or initial_data["complete"] is not False):
            reasons.append("beast telemetry: snake did not enter a clean playing state after Play")
        reasons.extend(_snake_move_transition(initial_data, down_data, "ordinary Down turn", (0, 1)))
        reasons.extend(_snake_move_transition(down_data, right_data, "ordinary Right turn", (1, 0)))
        reasons.extend(_snake_feed_setup_transition(right_data, feed_setup, "feed action"))
        reasons.extend(_snake_feed_transition(feed_setup, fed_data, "food tick"))

        if fed_data is not None and collision_setup is not None:
            reasons.extend(_snake_unchanged(
                fed_data,
                collision_setup,
                "collide action",
                ("score", "length", "steps", "foods_eaten", "deaths", "target_foods"),
            ))
            if collision_setup["state"] != "playing" or collision_setup["complete"] is not False:
                reasons.append("beast telemetry: collide action became terminal before the shared collision tick")
        if collision_setup is not None and collision_data is not None:
            if collision_data["state"] not in {"lost", "gameover", "game_over"} or collision_data["complete"] is not False:
                reasons.append("beast telemetry: shared collision tick did not enter a real loss state")
            if collision_data["metrics"].get("deaths") != collision_setup["metrics"].get("deaths", -1) + 1:
                reasons.append("beast telemetry: shared collision tick did not increment deaths exactly once")
            for name in ("length", "steps", "foods_eaten", "target_foods"):
                if collision_data["metrics"].get(name) != collision_setup["metrics"].get(name):
                    reasons.append(f"beast telemetry: shared collision tick changed unrelated metrics.{name}")
            if (
                collision_data["score"] != collision_setup["score"]
                or collision_data["body"] != collision_setup["body"]
                or collision_data["actor"] != collision_setup["actor"]
                or collision_data["food"] != collision_setup["food"]
            ):
                reasons.append("beast telemetry: shared collision tick rewrote score/body/food instead of only losing")

        if restart_data is not None:
            if restart_data["state"] != "playing" or restart_data["complete"] is not False:
                reasons.append("beast telemetry: visible restart did not return snake to playing")
            if restart_data["score"] != 0:
                reasons.append("beast telemetry: visible restart did not reset score")
            for name in ("steps", "foods_eaten", "deaths"):
                if restart_data["metrics"].get(name) != 0:
                    reasons.append(f"beast telemetry: visible restart did not reset metrics.{name}")
            if initial_data is not None:
                for name in ("length", "target_foods"):
                    if restart_data["metrics"].get(name) != initial_data["metrics"].get(name):
                        reasons.append(f"beast telemetry: visible restart did not restore metrics.{name}")

        setup_items = telemetry.get("progressionSetups") or []
        transition_items = telemetry.get("transitions") or []
        if len(setup_items) != len(transition_items) or not transition_items:
            reasons.append("beast telemetry: snake finite progression did not produce paired setup/tick snapshots")
        current = restart_data
        final_progress = None
        for index, (setup_item, transition_item) in enumerate(zip(setup_items, transition_items)):
            setup_data, setup_reasons = _snake_snapshot(setup_item, f"progression feed setup {index + 1}")
            after_data, after_reasons = _snake_snapshot(transition_item, f"progression feed tick {index + 1}")
            reasons.extend(setup_reasons + after_reasons)
            if actions.get(f"progression_feed_{index}") is not True:
                reasons.append(f"beast telemetry: progression feed {index + 1} did not report a successful setup")
            reasons.extend(_snake_feed_setup_transition(current, setup_data, f"progression feed action {index + 1}"))
            reasons.extend(_snake_feed_transition(setup_data, after_data, f"progression food tick {index + 1}"))
            if after_data is not None:
                current = after_data
                final_progress = after_data
        if session == "finite":
            if (
                final_progress is None
                or final_progress["complete"] is not True
                or final_progress["state"] not in {"won", "win", "complete", "completed"}
                or final_progress["metrics"].get("foods_eaten", -1) < final_progress["metrics"].get("target_foods", 1)
            ):
                reasons.append("beast telemetry: bounded real food ticks never reached the finite snake win")
    elif expected_profile == "endless_runner":
        jump_snap = telemetry.get("duringJump")
        y0 = _first_num(_num(initial, "actor", "y"), _num(initial, "player", "y"))
        y1 = _first_num(_num(jump_snap, "actor", "y"), _num(jump_snap, "player", "y"))
        vy1 = _first_num(_num(jump_snap, "actor", "vy"), _num(jump_snap, "player", "vy"))
        if y0 is None or y1 is None or vy1 is None or not (y1 < y0 - 0.5 or vy1 < -0.25 or vy1 > 0.25):
            reasons.append("beast telemetry: Space/Up did not produce an upward jump")
        posture = moved.get("metrics", {}).get("posture") if moved else None
        if posture is None:
            posture = moved.get("posture") if moved else None
        if posture is None:
            reasons.append("beast telemetry: holding Down did not set metrics.posture")
        lives0 = _first_num(_num(initial, "metrics", "lives"), _num(initial, "lives"))
        lives1 = _first_num(_num(damaged, "metrics", "lives"), _num(damaged, "lives"))
        if lives0 is None or lives1 is None or lives1 >= lives0:
            reasons.append("beast telemetry: damage action did not reduce lives")
        if transitions:
            term = transitions[-1]
            state = term.get("state")
            complete = term.get("complete")
            if not (complete is True or (state and state in {"lost", "gameover", "game_over"})):
                reasons.append("beast telemetry: fail action did not trigger a terminal gameover state")
        else:
            reasons.append("beast telemetry: fail action produced no progression snapshots")
    elif expected_profile == "shooter":
        before_fire = telemetry.get("beforeFire")
        after_fire = telemetry.get("afterFire")
        actions = report.get("actionResults") or {}

        x0 = _num(initial, "actor", "x")
        y0 = _num(initial, "actor", "y")
        x1 = _num(moved, "actor", "x")
        y1 = _num(moved, "actor", "y")
        if None in {x0, y0, x1, y1} or (x0 == x1 and y0 == y1):
            reasons.append("beast telemetry: ordinary directional input did not move shooter actor")

        projectiles0 = _num(before_fire, "metrics", "projectiles")
        projectiles1 = _num(after_fire, "metrics", "projectiles")
        if projectiles0 is None or projectiles1 is None or projectiles1 <= projectiles0:
            reasons.append("beast telemetry: ordinary Space input did not create a real projectile")

        if actions.get("damage") is not True:
            reasons.append("beast telemetry: damage action did not report a real successful transition")
        lives0 = _num(after_fire, "metrics", "lives")
        lives1 = _num(damaged, "metrics", "lives")
        if lives0 is None or lives1 is None or lives1 >= lives0:
            reasons.append("beast telemetry: damage action did not reduce real shooter lives")

        if actions.get("advance") is not True:
            reasons.append("beast telemetry: advance action did not report a real successful transition")
        advanced = transitions[-1] if transitions else None
        wave0 = _num(damaged, "metrics", "wave")
        wave1 = _num(advanced, "metrics", "wave")
        if not _terminal(advanced) and (wave0 is None or wave1 is None or wave1 <= wave0):
            reasons.append("beast telemetry: advance action did not increase shooter wave or reach victory")
    elif expected_profile == "breakout_pinball":
        moved_ball = telemetry.get("afterRight") or {}
        hit = telemetry.get("afterHit") or {}
        x0 = _num(initial, "metrics", "ball", "x")
        y0 = _num(initial, "metrics", "ball", "y")
        x1 = _num(moved_ball, "metrics", "ball", "x")
        y1 = _num(moved_ball, "metrics", "ball", "y")
        if None in {x0, y0, x1, y1} or (x0 == x1 and y0 == y1):
            reasons.append("beast telemetry: launched ball did not move")
        targets0 = _num(initial, "metrics", "targets")
        targets1 = _num(hit, "metrics", "targets")
        if targets0 is None or targets1 is None or targets1 >= targets0:
            reasons.append("beast telemetry: hit_target action did not reduce metrics.targets")
        balls0 = _num(initial, "metrics", "balls")
        balls1 = _num(damaged, "metrics", "balls")
        drain_state = damaged.get("state") if damaged else None
        if (
            (balls0 is None or balls1 is None or balls1 >= balls0)
            and drain_state not in {"lost", "gameover", "game_over"}
        ):
            reasons.append("beast telemetry: drain action did not lose a ball or reach terminal state")
        level0 = _num(initial, "metrics", "level")
        level1 = _num(transitions[-1], "metrics", "level") if transitions else None
        if level0 is None or level1 is None or level1 <= level0:
            reasons.append("beast telemetry: advance action did not increase metrics.level")
    elif expected_profile == "puzzle_grid":
        moved_candidates = [
            item for item in (telemetry.get("afterMove"), telemetry.get("afterRight"))
            if isinstance(item, dict)
        ]
        pushed_candidates = [
            item for item in (telemetry.get("afterPush"), telemetry.get("afterRight"))
            if isinstance(item, dict)
        ]
        x0 = _num(initial, "actor", "x")
        y0 = _num(initial, "actor", "y")
        moved = any(
            None not in {x0, y0, _num(item, "actor", "x"), _num(item, "actor", "y")}
            and (x0 != _num(item, "actor", "x") or y0 != _num(item, "actor", "y"))
            for item in moved_candidates
        )
        if not moved:
            reasons.append("beast telemetry: move action did not change actor grid position")
        moves0 = _num(initial, "metrics", "moves")
        if moves0 is None or not any(
            (value := _num(item, "metrics", "moves")) is not None and value > moves0
            for item in moved_candidates
        ):
            reasons.append("beast telemetry: move action did not increase metrics.moves")
        goals0 = _num(initial, "metrics", "crates_on_goals")
        if goals0 is None or not any(
            (value := _num(item, "metrics", "crates_on_goals")) is not None and value > goals0
            for item in pushed_candidates
        ):
            reasons.append("beast telemetry: push action did not increase metrics.crates_on_goals")
        level0 = _num(initial, "metrics", "level")
        level1 = _num(transitions[-1], "metrics", "level") if transitions else None
        if level0 is None or level1 is None or level1 <= level0:
            reasons.append("beast telemetry: advance action did not increase metrics.level")
    elif expected_profile == "match3":
        pointer = telemetry.get("afterPointer")
        swapped = telemetry.get("afterSwap")
        initial_data, initial_reasons = _match3_snapshot(initial, "initial")
        pointer_data, pointer_reasons = _match3_snapshot(pointer, "pointer swap")
        swap_data, swap_reasons = _match3_snapshot(swapped, "adapter swap")
        reasons.extend(initial_reasons + pointer_reasons + swap_reasons)

        def require_real_swap(before: dict[str, Any] | None, after: dict[str, Any] | None, label: str) -> None:
            if before is None or after is None:
                return
            if after["hash"] == before["hash"]:
                reasons.append(f"beast telemetry: {label} did not change the canonical board")
            before_moves = _num({"metrics": before["metrics"]}, "metrics", "moves")
            after_moves = _num({"metrics": after["metrics"]}, "metrics", "moves")
            if before_moves is None or after_moves is None or after_moves != before_moves + 1:
                reasons.append(f"beast telemetry: {label} did not increment total moves exactly once")
            before_cascades = _num({"metrics": before["metrics"]}, "metrics", "cascades")
            after_cascades = _num({"metrics": after["metrics"]}, "metrics", "cascades")
            if before_cascades is None or after_cascades is None or after_cascades <= before_cascades:
                reasons.append(f"beast telemetry: {label} did not earn a real cascade")
            before_score = before.get("score")
            after_score = after.get("score")
            if not isinstance(before_score, (int, float)) or not isinstance(after_score, (int, float)) or after_score <= before_score:
                reasons.append(f"beast telemetry: {label} did not increase real score")

        require_real_swap(initial_data, pointer_data, "ordinary pointer swap")
        require_real_swap(pointer_data, swap_data, "legal_swap action")

        level0 = _num(initial, "metrics", "level")
        completed0 = _num(initial, "metrics", "completed_levels")
        total0 = _num(initial, "metrics", "total_levels")
        target0 = _num(initial, "metrics", "target_score")
        if total0 is None or total0 < 2 or target0 is None or target0 <= 0:
            reasons.append("beast telemetry: match3 finite level/target contract invalid")
        progressed = None
        for item in transitions:
            item_data, item_reasons = _match3_snapshot(item, "progression")
            reasons.extend(item_reasons)
            item_level = _num(item, "metrics", "level")
            if _terminal(item) or (level0 is not None and item_level is not None and item_level > level0):
                progressed = (item, item_data)
                break
        if progressed is None:
            reasons.append("beast telemetry: bounded earned match3 swaps never advanced a finite level")
        else:
            final, final_data = progressed
            completed1 = _num(final, "metrics", "completed_levels")
            total1 = _num(final, "metrics", "total_levels")
            if completed0 is None or completed1 is None or completed1 <= completed0:
                reasons.append("beast telemetry: earned level progress did not increase completed_levels")
            if total0 is None or total1 is None or total1 != total0:
                reasons.append("beast telemetry: total_levels changed during progression")
            if final_data is not None and initial_data is not None and final_data["hash"] == initial_data["hash"]:
                reasons.append("beast telemetry: earned level progress did not install a new board")
            if not _terminal(final):
                level_moves = _num(final, "metrics", "level_moves")
                if level_moves is None or level_moves != 0:
                    reasons.append("beast telemetry: new match3 level did not reset level_moves")
    elif expected_profile == "tower_defense":
        snapshots = {
            "initial": initial,
            "ordinary pointer placement": telemetry.get("afterPointer"),
            "adapter placement": telemetry.get("afterPlace"),
            "wave start": telemetry.get("afterWaveStart"),
            "combat": telemetry.get("afterCombat"),
            "wave progress": telemetry.get("afterWaveProgress"),
            "before leak": telemetry.get("beforeLeak"),
            "after leak": telemetry.get("afterLeak"),
        }
        defense: dict[str, dict[str, Any] | None] = {}
        for label, item in snapshots.items():
            data, snapshot_reasons = _tower_defense_snapshot(item, label)
            defense[label] = data
            reasons.extend(snapshot_reasons)

        actions = report.get("actionResults") or {}
        for action in ("place_tower", "start_wave", "start_wave_for_leak", "leak"):
            if actions.get(action) is not True:
                reasons.append(f"beast telemetry: {action} action did not report a real successful transition")

        initial_data = defense["initial"]
        pointer_data = defense["ordinary pointer placement"]
        placed_data = defense["adapter placement"]
        wave_data = defense["wave start"]
        combat_data = defense["combat"]
        progress_data = defense["wave progress"]
        before_leak_data = defense["before leak"]
        after_leak_data = defense["after leak"]
        reasons.extend(_tower_placement_transition(initial_data, pointer_data, "ordinary pointer placement"))
        reasons.extend(_tower_placement_transition(pointer_data, placed_data, "place_tower action"))

        if placed_data is not None and wave_data is not None:
            before_metrics, after_metrics = placed_data["metrics"], wave_data["metrics"]
            if _num(after_metrics, "spawned") is None or _num(after_metrics, "spawned") <= (_num(before_metrics, "spawned") or 0):
                reasons.append("beast telemetry: start_wave did not spawn a real enemy")
            if not wave_data["enemy_ids"] or not (wave_data["enemy_ids"] - placed_data["enemy_ids"]):
                reasons.append("beast telemetry: start_wave did not add a stable-ID live enemy")
            for name in ("currency", "base_hp", "spent", "earned", "kills", "leaks", "completed_waves", "wave"):
                if _num(after_metrics, name) != _num(before_metrics, name):
                    reasons.append(f"beast telemetry: start_wave changed unrelated metrics.{name}")
            if wave_data["hash"] != placed_data["hash"] or wave_data["tower_ids"] != placed_data["tower_ids"]:
                reasons.append("beast telemetry: start_wave rewrote the tower board or roster")

        if wave_data is not None and combat_data is not None:
            before_metrics, after_metrics = wave_data["metrics"], combat_data["metrics"]
            for name in ("shots", "hits", "kills"):
                if _num(after_metrics, name) is None or _num(after_metrics, name) <= (_num(before_metrics, name) or 0):
                    reasons.append(f"beast telemetry: tower combat did not increase metrics.{name}")
            removed_enemies = wave_data["enemy_ids"] - combat_data["enemy_ids"]
            if not removed_enemies:
                reasons.append("beast telemetry: tower combat did not remove a pre-existing stable-ID enemy")
            earned_delta = (_num(after_metrics, "earned") or 0) - (_num(before_metrics, "earned") or 0)
            currency_delta = (_num(after_metrics, "currency") or 0) - (_num(before_metrics, "currency") or 0)
            if earned_delta <= 0 or currency_delta != earned_delta:
                reasons.append("beast telemetry: a real kill did not award matching earned currency")
            if _num(after_metrics, "leaks") != _num(before_metrics, "leaks"):
                reasons.append("beast telemetry: combat proof also leaked an enemy")
            if _num(after_metrics, "base_hp") != _num(before_metrics, "base_hp"):
                reasons.append("beast telemetry: combat proof changed base_hp without a leak")
            if combat_data["hash"] != wave_data["hash"] or combat_data["tower_ids"] != wave_data["tower_ids"]:
                reasons.append("beast telemetry: combat rewrote the tower board or roster")

        if placed_data is not None and progress_data is not None:
            before_metrics, after_metrics = placed_data["metrics"], progress_data["metrics"]
            completed_before = _num(before_metrics, "completed_waves")
            completed_after = _num(after_metrics, "completed_waves")
            if completed_before is None or completed_after != completed_before + 1:
                reasons.append("beast telemetry: earned wave clear did not increment completed_waves exactly once")
            wave_before = _num(before_metrics, "wave")
            wave_after = _num(after_metrics, "wave")
            if not _terminal(progress_data["snapshot"]) and (wave_before is None or wave_after != wave_before + 1):
                reasons.append("beast telemetry: earned wave clear did not advance the finite wave exactly once")
            if _num(after_metrics, "total_waves") != _num(before_metrics, "total_waves"):
                reasons.append("beast telemetry: total_waves changed during progression")
            if (_num(after_metrics, "kills") or 0) <= (_num(before_metrics, "kills") or 0):
                reasons.append("beast telemetry: wave advanced without an earned enemy kill")

        if progress_data is not None and before_leak_data is not None:
            before_metrics, after_metrics = progress_data["metrics"], before_leak_data["metrics"]
            if _num(after_metrics, "spawned") is None or _num(after_metrics, "spawned") <= (_num(before_metrics, "spawned") or 0):
                reasons.append("beast telemetry: second start_wave did not spawn an enemy for leak proof")
            if not before_leak_data["enemy_ids"]:
                reasons.append("beast telemetry: leak proof began without a live enemy")

        if before_leak_data is not None and after_leak_data is not None:
            before_metrics, after_metrics = before_leak_data["metrics"], after_leak_data["metrics"]
            leaks_before = _num(before_metrics, "leaks")
            leaks_after = _num(after_metrics, "leaks")
            if leaks_before is None or leaks_after != leaks_before + 1:
                reasons.append("beast telemetry: leak action did not increment real leaks exactly once")
            hp_before = _num(before_metrics, "base_hp")
            hp_after = _num(after_metrics, "base_hp")
            if hp_before is None or hp_after is None or hp_after >= hp_before:
                reasons.append("beast telemetry: leak action did not reduce real base_hp")
            if not (before_leak_data["enemy_ids"] - after_leak_data["enemy_ids"]):
                reasons.append("beast telemetry: leak action did not consume a pre-existing stable-ID enemy")
            if after_leak_data["hash"] != before_leak_data["hash"] or after_leak_data["tower_ids"] != before_leak_data["tower_ids"]:
                reasons.append("beast telemetry: leak action rewrote the tower board or roster")
    elif expected_profile == "rhythm":
        snapshots = {
            "initial": initial,
            "before ordinary lane input": telemetry.get("beforeOrdinary"),
            "after ordinary lane input": telemetry.get("afterOrdinary"),
            "before hit_next": telemetry.get("beforeHit"),
            "after hit_next": telemetry.get("afterHit"),
            "before miss_next": telemetry.get("beforeMiss"),
            "after miss_next": telemetry.get("afterMiss"),
            "before finish": telemetry.get("beforeFinish"),
            "after finish": telemetry.get("afterFinish"),
            "after restart": telemetry.get("afterRestart"),
        }
        rhythm: dict[str, dict[str, Any] | None] = {}
        for label, item in snapshots.items():
            data, snapshot_reasons = _rhythm_snapshot(item, label)
            rhythm[label] = data
            reasons.extend(snapshot_reasons)

        actions = report.get("actionResults") or {}
        for action in ("hit_next", "miss_next", "finish"):
            if actions.get(action) is not True:
                reasons.append(f"beast telemetry: {action} action did not report a real successful transition")
        if int(report.get("audioStartsAfterPlay") or 0) < 1:
            reasons.append("rhythm track audio did not start from the Play gesture")
        if report.get("clickedRestart") is not True:
            reasons.append("rhythm results did not expose a visible Restart/Retry action")

        initial_data = rhythm["initial"]
        before_ordinary = rhythm["before ordinary lane input"]
        after_ordinary = rhythm["after ordinary lane input"]
        before_hit = rhythm["before hit_next"]
        after_hit = rhythm["after hit_next"]
        before_miss = rhythm["before miss_next"]
        after_miss = rhythm["after miss_next"]
        before_finish = rhythm["before finish"]
        after_finish = rhythm["after finish"]
        after_restart = rhythm["after restart"]

        if initial_data is not None:
            start_snapshot = initial_data["snapshot"]
            start_metrics = initial_data["metrics"]
            if (
                start_snapshot.get("state") != "playing"
                or start_snapshot.get("complete") is not False
                or start_snapshot.get("score") != 0
                or any(judgement is not None for judgement in initial_data["statuses"].values())
            ):
                reasons.append("beast telemetry: Play did not install a clean pending rhythm chart")
            for name in ("combo", "max_combo", "hits", "perfect", "good", "misses", "judged_notes"):
                if _num(start_metrics, name) != 0:
                    reasons.append(f"beast telemetry: Play did not reset metrics.{name}")
            if _num(start_metrics, "remaining_notes") != _num(start_metrics, "total_notes"):
                reasons.append("beast telemetry: Play did not expose every chart note as pending")

        if initial_data is not None and before_ordinary is not None:
            if initial_data["track"] != before_ordinary["track"] or initial_data["hash"] != before_ordinary["hash"]:
                reasons.append("beast telemetry: song clock wait rewrote the immutable rhythm chart")
            clock0 = _num(initial_data["metrics"], "clock_ms")
            clock1 = _num(before_ordinary["metrics"], "clock_ms")
            if clock0 is None or clock1 is None or clock1 <= clock0:
                reasons.append("beast telemetry: rhythm song clock did not progress after Play")
            ordinary = report.get("ordinaryLaneInput")
            hint = before_ordinary["next_note"]
            expected_key = hint.get("key") if isinstance(hint, dict) else None
            expected_code = "Key" + str(expected_key).upper() if isinstance(expected_key, str) else None
            if not (
                isinstance(ordinary, dict) and isinstance(hint, dict)
                and ordinary.get("note_id") == hint.get("id")
                and ordinary.get("lane") == hint.get("lane")
                and ordinary.get("key") == expected_key
                and ordinary.get("code") == expected_code
            ):
                reasons.append("beast telemetry: ordinary lane input did not dispatch the lowercase key for next_note")

        reasons.extend(_rhythm_single_transition(before_ordinary, after_ordinary, "ordinary lane input", expected="hit"))
        reasons.extend(_rhythm_single_transition(before_hit, after_hit, "hit_next action", expected="hit"))
        reasons.extend(_rhythm_single_transition(before_miss, after_miss, "miss_next action", expected="miss"))

        if before_finish is not None and after_finish is not None:
            if (
                before_finish["track"] != after_finish["track"]
                or before_finish["identities"] != after_finish["identities"]
                or before_finish["hash"] != after_finish["hash"]
            ):
                reasons.append("beast telemetry: finish action rewrote the immutable rhythm chart")
            finish_changes = [
                (note_id, judgement, after_finish["statuses"].get(note_id))
                for note_id, judgement in before_finish["statuses"].items()
                if after_finish["statuses"].get(note_id) != judgement
            ]
            pending_before = sum(1 for judgement in before_finish["statuses"].values() if judgement is None)
            if (
                pending_before < 1
                or len(finish_changes) != pending_before
                or any(old is not None or new != "miss" for _, old, new in finish_changes)
            ):
                reasons.append("beast telemetry: finish did not resolve every remaining real note through the miss path")
            old, new = before_finish["metrics"], after_finish["metrics"]
            if _num(new, "remaining_notes") != 0 or _num(new, "judged_notes") != _num(new, "total_notes"):
                reasons.append("beast telemetry: finish reached results without judging the full chart")
            if _num(new, "misses") != (_num(old, "misses") or 0) + pending_before:
                reasons.append("beast telemetry: finish miss accounting does not equal the remaining chart")
            for name in ("hits", "perfect", "good", "max_combo"):
                if _num(new, name) != _num(old, name):
                    reasons.append(f"beast telemetry: finish changed unrelated metrics.{name}")
            if after_finish["score"] != before_finish["score"] or _num(new, "combo") != 0:
                reasons.append("beast telemetry: finish awarded score or preserved combo for forced misses")
            final_snapshot = after_finish["snapshot"]
            if final_snapshot.get("complete") is not True or not _terminal(final_snapshot):
                reasons.append("beast telemetry: fully judged rhythm chart did not enter terminal results")

        if initial_data is not None and after_restart is not None:
            if (
                initial_data["track"] != after_restart["track"]
                or initial_data["identities"] != after_restart["identities"]
                or initial_data["hash"] != after_restart["hash"]
            ):
                reasons.append("beast telemetry: restart did not restore the same immutable rhythm chart")
            restart_snapshot = after_restart["snapshot"]
            restart_metrics = after_restart["metrics"]
            if (
                restart_snapshot.get("state") != "playing"
                or restart_snapshot.get("complete") is not False
                or restart_snapshot.get("score") != 0
                or any(judgement is not None for judgement in after_restart["statuses"].values())
                or (_num(restart_metrics, "clock_ms") or 0) > 250
            ):
                reasons.append("beast telemetry: visible restart did not install a clean playing track")
            for name in ("combo", "max_combo", "hits", "perfect", "good", "misses", "judged_notes"):
                if _num(restart_metrics, name) != 0:
                    reasons.append(f"beast telemetry: restart did not reset metrics.{name}")
            if _num(restart_metrics, "remaining_notes") != _num(restart_metrics, "total_notes"):
                reasons.append("beast telemetry: restart did not restore every pending note")
    elif expected_profile == "frogger":
        x0 = _num(initial, "actor", "x")
        y0 = _num(initial, "actor", "y")
        x1 = _num(moved, "actor", "x")
        y1 = _num(moved, "actor", "y")
        if None in {x0, y0, x1, y1} or (x0 == x1 and y0 == y1):
            reasons.append("beast telemetry: ordinary directional hop did not move actor")
        lives0 = _num(initial, "metrics", "lives")
        lives1 = _num(damaged, "metrics", "lives")
        if lives0 is None or lives1 is None or lives1 >= lives0:
            reasons.append("beast telemetry: collide action did not reduce real lives")
        goals0 = _num(initial, "metrics", "goals")
        level0 = _num(initial, "metrics", "level")
        final = transitions[-1] if transitions else None
        goals1 = _num(final, "metrics", "goals")
        level1 = _num(final, "metrics", "level")
        if goals0 is None or goals1 is None or goals1 <= goals0:
            reasons.append("beast telemetry: reach_goal action did not increase earned goals")
        if not _terminal(final) and (level0 is None or level1 is None or level1 <= level0):
            reasons.append("beast telemetry: bounded reach_goal actions did not advance level")

    if requested_profile == "snake":
        if int(report.get("audioStartsAfterPlay") or 0) < 1:
            reasons.append("snake audio did not start from the Play gesture")
    elif int(report.get("audioStarts") or 0) < 1:
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

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import readline from 'node:readline/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const appDir = path.dirname(fileURLToPath(import.meta.url));
const currentFile = path.join(appDir, 'current.strudel');
const stateFile = path.join(appDir, 'state.json');
const historyFile = path.join(appDir, 'history.jsonl');

function suppressStrudelNoise() {
  const originalLog = console.log;
  const originalWarn = console.warn;
  console.log = (...args) => {
    const msg = args.join(' ');
    if (msg.includes('@strudel/core loaded')) return;
    originalLog(...args);
  };
  console.warn = (...args) => {
    const msg = args.join(' ');
    if (msg.includes('cannot use window')) return;
    originalWarn(...args);
  };
  return () => {
    console.log = originalLog;
    console.warn = originalWarn;
  };
}

async function loadStrudel() {
  const restore = suppressStrudelNoise();
  try {
    const core = await import('@strudel/core');
    const mini = await import('@strudel/mini');
    const tonal = await import('@strudel/tonal');
    const { transpiler } = await import('@strudel/transpiler');
    await core.evalScope(core, mini, tonal);
    return { core, transpiler };
  } finally {
    restore();
  }
}

function stripCodeEnvelope(input) {
  let code = input.trim();
  const tagMatch = code.match(/^\[STRUDEL_MUSIC:\s*([\s\S]*)\]\s*$/i);
  if (tagMatch) code = tagMatch[1].trim();
  const fenceMatch = code.match(/^```(?:strudel|javascript|js)?\s*([\s\S]*?)\s*```\s*$/i);
  if (fenceMatch) code = fenceMatch[1].trim();
  return code;
}

function readStdin() {
  return new Promise((resolve, reject) => {
    let data = '';
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', chunk => {
      data += chunk;
    });
    process.stdin.on('end', () => resolve(data));
    process.stdin.on('error', reject);
  });
}

function numberValue(value) {
  if (typeof value?.valueOf === 'function') return Number(value.valueOf());
  return Number(value);
}

function serializeEvent(event) {
  return {
    begin: numberValue(event.whole.begin),
    end: numberValue(event.whole.end),
    beginFraction: event.whole.begin.toFraction?.() ?? String(event.whole.begin),
    endFraction: event.whole.end.toFraction?.() ?? String(event.whole.end),
    value: event.value,
    show: typeof event.show === 'function' ? event.show() : JSON.stringify(event.value),
  };
}

const DEPTH_CONTROLS = new Set([
  'gain', 'pan', 'lpf', 'cutoff', 'resonance', 'lpq', 'room', 'size',
  'delay', 'delaytime', 'delayfeedback', 'attack', 'decay', 'sustain',
  'release', 'speed', 'orbit',
]);

function eventIdentity(event) {
  const value = event.value ?? {};
  const payload = {
    at: Number((event.begin - Math.floor(event.begin)).toFixed(4)),
    dur: Number((event.end - event.begin).toFixed(4)),
    note: value.note ?? null,
    sample: value.s ?? null,
  };
  for (const key of Object.keys(value).sort()) {
    if (DEPTH_CONTROLS.has(key)) payload[key] = value[key];
  }
  return payload;
}

const NOTE_PC = new Map([
  ['c', 0], ['c#', 1], ['db', 1], ['d', 2], ['d#', 3], ['eb', 3],
  ['e', 4], ['fb', 4], ['e#', 5], ['f', 5], ['f#', 6], ['gb', 6],
  ['g', 7], ['g#', 8], ['ab', 8], ['a', 9], ['a#', 10], ['bb', 10],
  ['b', 11], ['cb', 11],
]);

const TONAL_MODES = new Map([
  ['major', [0, 2, 4, 5, 7, 9, 11]],
  ['natural_minor', [0, 2, 3, 5, 7, 8, 10]],
  ['dorian', [0, 2, 3, 5, 7, 9, 10]],
  ['mixolydian', [0, 2, 4, 5, 7, 9, 10]],
  ['phrygian', [0, 1, 3, 5, 7, 8, 10]],
  ['harmonic_minor', [0, 2, 3, 5, 7, 8, 11]],
]);

function noteToMidi(note) {
  const match = String(note ?? '').trim().toLowerCase().match(/^([a-g])([#b]?)(-?\d+)$/);
  if (!match) return null;
  const pc = NOTE_PC.get(match[1] + match[2]);
  return pc === undefined ? null : (Number(match[3]) + 1) * 12 + pc;
}

function median(values) {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function voiceIdentity(value) {
  const identity = {};
  for (const key of ['s', ...DEPTH_CONTROLS].sort()) {
    if (value[key] !== undefined) identity[key] = value[key];
  }
  return JSON.stringify(identity);
}

function analyzeMusicalCoherence(events, cycles) {
  const pitched = events
    .map(event => ({ ...event, midi: noteToMidi(event.value?.note) }))
    .filter(event => event.midi !== null);
  if (!pitched.length) return {};

  const pitchWeights = Array(12).fill(0);
  const voices = new Map();
  let maxIndividualGain = 0;
  for (const event of pitched) {
    const value = event.value ?? {};
    const gain = Math.max(0, Number(value.gain ?? 0.5));
    const duration = Math.max(0.01, Math.min(1, event.end - event.begin));
    pitchWeights[event.midi % 12] += duration * Math.sqrt(Math.max(gain, 0.01));
    maxIndividualGain = Math.max(maxIndividualGain, gain);
    const key = voiceIdentity(value);
    if (!voices.has(key)) voices.set(key, []);
    voices.get(key).push(event);
  }

  const totalPitchWeight = pitchWeights.reduce((a, b) => a + b, 0);
  let bestScale = { fit: 0, tonic: 0, mode: 'major' };
  for (let tonic = 0; tonic < 12; tonic += 1) {
    for (const [mode, intervals] of TONAL_MODES) {
      const allowed = new Set(intervals.map(interval => (tonic + interval) % 12));
      const fit = pitchWeights.reduce((sum, weight, pc) => sum + (allowed.has(pc) ? weight : 0), 0)
        / Math.max(totalPitchWeight, 1e-9);
      if (fit > bestScale.fit) bestScale = { fit, tonic, mode };
    }
  }

  const registerBands = new Set();
  const voiceMedians = [];
  const onsetSignatures = new Set();
  const bins = Math.max(32, cycles * 8);
  const occupancy = Array.from({ length: bins }, () => new Set());
  const cycleNoteCounts = Array(cycles).fill(0);
  for (const [voice, voiceEvents] of voices) {
    const center = median(voiceEvents.map(event => event.midi));
    voiceMedians.push(Number(center.toFixed(2)));
    registerBands.add(center < 48 ? 'low' : center < 67 ? 'middle' : 'high');
    const onsets = new Set();
    for (const event of voiceEvents) {
      const cycle = Math.floor(event.begin);
      if (cycle >= 0 && cycle < cycles) cycleNoteCounts[cycle] += 1;
      const onsetBin = Math.max(0, Math.min(bins - 1, Math.floor((event.begin / cycles) * bins)));
      onsets.add(onsetBin);
      const first = onsetBin;
      const last = Math.max(first, Math.min(bins - 1, Math.ceil((event.end / cycles) * bins) - 1));
      for (let bin = first; bin <= last; bin += 1) occupancy[bin].add(voice);
    }
    onsetSignatures.add([...onsets].sort((a, b) => a - b).join(','));
  }

  const voiceCount = voices.size;
  const activeVoiceCounts = occupancy.map(set => set.size);
  const fullDensityFraction = activeVoiceCounts.filter(count => count >= voiceCount).length / bins;
  const breathThreshold = Math.max(1, voiceCount - 2);
  const breathingRoomFraction = activeVoiceCounts.filter(count => count <= breathThreshold).length / bins;
  const liveCycleCounts = cycleNoteCounts.filter(Boolean);
  const densityContrast = liveCycleCounts.length
    ? Math.max(...liveCycleCounts) / Math.max(1, Math.min(...liveCycleCounts))
    : 0;

  return {
    pitchedVoices: voiceCount,
    bestScaleFit: Number(bestScale.fit.toFixed(4)),
    inferredTonicPc: bestScale.tonic,
    inferredMode: bestScale.mode,
    pitchClassCount: pitchWeights.filter(weight => weight > 0).length,
    registerBands: [...registerBands].sort(),
    voiceMedianMidi: voiceMedians.sort((a, b) => a - b),
    distinctVoiceRhythms: onsetSignatures.size,
    breathingRoomFraction: Number(breathingRoomFraction.toFixed(4)),
    fullDensityFraction: Number(fullDensityFraction.toFixed(4)),
    pitchedDensityContrast: Number(densityContrast.toFixed(4)),
    maxIndividualGain: Number(maxIndividualGain.toFixed(4)),
  };
}

function analyzeEvents(events, cycles) {
  const notes = new Set();
  const samples = new Set();
  const controls = new Set();
  const cycleEvents = Array.from({ length: cycles }, () => []);
  let noteEvents = 0;
  let sampleEvents = 0;

  for (const event of events) {
    const value = event.value ?? {};
    if (value.note !== undefined) {
      noteEvents += 1;
      notes.add(String(value.note));
    } else if (value.s !== undefined) {
      sampleEvents += 1;
      samples.add(String(value.s));
    }
    for (const key of Object.keys(value)) {
      if (DEPTH_CONTROLS.has(key)) controls.add(key);
    }
    const cycle = Math.floor(event.begin);
    if (cycle >= 0 && cycle < cycles) cycleEvents[cycle].push(eventIdentity(event));
  }

  const cycleCounts = cycleEvents.map(items => items.length);
  const signatures = cycleEvents
    .filter(items => items.length > 0)
    .map(items => JSON.stringify(items.sort((a, b) => a.at - b.at || String(a.note ?? a.sample).localeCompare(String(b.note ?? b.sample)))));

  const activeCounts = cycleCounts.filter(Boolean);
  return {
    eventCount: events.length,
    noteEvents,
    sampleEvents,
    uniqueNotes: [...notes].sort(),
    uniqueSamples: [...samples].sort(),
    controls: [...controls].sort(),
    activeCycles: cycleCounts.filter(Boolean).length,
    cycleCounts,
    distinctCycleSignatures: new Set(signatures).size,
    minActiveCycleDensity: activeCounts.length ? Math.min(...activeCounts) : 0,
    peakCycleDensity: Math.max(...cycleCounts, 0),
    ...analyzeMusicalCoherence(events, cycles),
  };
}

async function evaluateCode(code, cycles = 8) {
  const { core, transpiler } = await loadStrudel();
  const result = await core.evaluate(code, transpiler);
  if (!result.pattern || typeof result.pattern.queryArc !== 'function') {
    throw new Error('Strudel code did not evaluate to a queryable pattern.');
  }
  const events = result.pattern
    .queryArc(0, cycles)
    .map(serializeEvent)
    .sort((a, b) => a.begin - b.begin || a.end - b.end);
  return {
    ok: true,
    updatedAt: new Date().toISOString(),
    cycles,
    eventCount: events.length,
    transpiled: result.meta?.output ?? null,
    code,
    events,
    analysis: analyzeEvents(events, cycles),
  };
}

function saveState(state) {
  fs.writeFileSync(stateFile, JSON.stringify(state, null, 2), 'utf8');
  fs.appendFileSync(
    historyFile,
    JSON.stringify({
      updatedAt: state.updatedAt,
      ok: state.ok,
      eventCount: state.eventCount,
      code: state.code,
      error: state.error,
    }) + '\n',
    'utf8',
  );
}

function readCurrentCode() {
  return fs.readFileSync(currentFile, 'utf8');
}

async function commandSet(args) {
  let code;
  if (args[0] === '--stdin') {
    code = await readStdin();
  } else if (args[0]) {
    const maybePath = path.resolve(process.cwd(), args[0]);
    code = fs.existsSync(maybePath) ? fs.readFileSync(maybePath, 'utf8') : args.join(' ');
  } else {
    code = readCurrentCode();
  }
  code = stripCodeEnvelope(code);
  fs.writeFileSync(currentFile, code + '\n', 'utf8');
  const state = await evaluateCode(code, 8);
  saveState(state);
  printHumanSummary('saved', state);
}

async function commandValidate(args) {
  const code = args[0] === '--stdin'
    ? stripCodeEnvelope(await readStdin())
    : args[0]
      ? stripCodeEnvelope(fs.readFileSync(path.resolve(process.cwd(), args[0]), 'utf8'))
      : readCurrentCode();
  const cycles = Number(args[1] || 8);
  try {
    const state = await evaluateCode(code, cycles);
    saveState(state);
    printHumanSummary('valid', state);
    console.log(`ANALYSIS:${JSON.stringify(state.analysis)}`);
  } catch (error) {
    const failed = {
      ok: false,
      updatedAt: new Date().toISOString(),
      code,
      error: String(error.stack || error),
    };
    saveState(failed);
    console.error(failed.error);
    process.exitCode = 1;
  }
}

async function commandRender(args) {
  const cycles = Number(args[0] || 4);
  const code = args[1] ? stripCodeEnvelope(fs.readFileSync(path.resolve(process.cwd(), args[1]), 'utf8')) : readCurrentCode();
  const state = await evaluateCode(code, cycles);
  saveState(state);
  printHumanSummary('rendered', state);
  for (const event of state.events.slice(0, 80)) {
    console.log(event.show);
  }
}

function pythonPath() {
  const local = path.resolve(appDir, '..', '.venv', 'Scripts', 'python.exe');
  return fs.existsSync(local) ? local : 'python';
}

async function commandPlay(args) {
  const cycles = Number(args[0] || 8);
  const code = readCurrentCode();
  const state = await evaluateCode(code, cycles);
  saveState(state);
  printHumanSummary('playing', state);
  const child = spawn(pythonPath(), [path.join(appDir, 'player.py'), stateFile], {
    cwd: path.resolve(appDir, '..'),
    stdio: 'inherit',
  });
  child.on('exit', code => {
    process.exitCode = code ?? 0;
  });
}

function commandDevices() {
  const py = pythonPath();
  // List only output devices, with hints for TELEDRA_AUDIO_DEVICE
  const code = `
import sounddevice as sd
print("=== Strudel / Teledra Audio Output Devices ===")
print("Set with:  $env:TELEDRA_AUDIO_DEVICE = 'partial name or index'")
print("           $env:TELEDRA_AUDIO_HOSTAPI = 'DirectSound'")
print()
hostapis = sd.query_hostapis()
for i, d in enumerate(sd.query_devices()):
    if d.get('max_output_channels', 0) > 0:
        ha = hostapis[d['hostapi']]['name'] if d['hostapi'] < len(hostapis) else '?'
        mark = '  <-- current default' if (isinstance(sd.default.device, (list,tuple)) and sd.default.device[1] == i) or sd.default.device == i else ''
        print(f"{i}: {d['name']} (out:{d['max_output_channels']}) [{ha}]{mark}")
print()
print("Common fix for 'no audio': $env:TELEDRA_AUDIO_DEVICE = 'Speakers (Realtek' ; node ... play")
`;
  const child = spawn(py, ["-c", code], {
    cwd: path.resolve(appDir, '..'),
    stdio: 'inherit',
  });
  child.on('exit', () => {});
}

function commandStatus() {
  if (!fs.existsSync(stateFile)) {
    console.log('No state.json yet. Run: node app.mjs validate');
    return;
  }
  const state = JSON.parse(fs.readFileSync(stateFile, 'utf8'));
  printHumanSummary(state.ok ? 'status' : 'failed', state);
  if (state.error) console.log(state.error);
}

function commandSkill() {
  console.log(`Teledra local Strudel skill:
- Write complete pattern code inside [STRUDEL_MUSIC: ...] or a fenced \`\`\`strudel block.
- Use one stack(...) with at least six layers and multi-cycle <...> variation.
- Shared audible controls: gain, pan, slow, lpf, room, delay, delaytime, delayfeedback, attack, and release.
- Use slow(0.5) to accelerate; avoid variables, $: lines, cat/seq wrappers, parameter strings, and browser-only syntax so the native Sketchpad agrees with validation.
- Save refinements to strudel_app/current.strudel, then validate locally with: node strudel_app/app.mjs validate
- Render events with: node strudel_app/app.mjs render 8
- Play the current local pattern with: node strudel_app/app.mjs play 8
- List audio outputs (to fix "no audio"): node strudel_app/app.mjs devices
- Edit strudel_app/current.strudel when refining an existing piece.
- This is local. Do not open or depend on the public Strudel website.`);
}

async function commandOpen() {
  console.log('Teledra Local Strudel App. Commands: show, set, validate, render [cycles], play [cycles], devices, skill, exit');
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  while (true) {
    const line = (await rl.question('strudel> ')).trim();
    if (!line) continue;
    if (line === 'exit' || line === 'quit') break;
    if (line === 'show') {
      console.log(readCurrentCode());
    } else if (line.startsWith('set ')) {
      const code = stripCodeEnvelope(line.slice(4));
      fs.writeFileSync(currentFile, code + '\n', 'utf8');
      const state = await evaluateCode(code, 8);
      saveState(state);
      printHumanSummary('saved', state);
    } else if (line === 'validate') {
      await commandValidate([]);
    } else if (line.startsWith('render')) {
      await commandRender(line.split(/\s+/).slice(1));
    } else if (line.startsWith('play')) {
      await commandPlay(line.split(/\s+/).slice(1));
    } else if (line === 'devices') {
      commandDevices();
    } else if (line === 'skill') {
      commandSkill();
    } else {
      console.log('Unknown command.');
    }
  }
  rl.close();
}

function printHumanSummary(label, state) {
  console.log(`[${label}] ok=${state.ok} events=${state.eventCount ?? 0} cycles=${state.cycles ?? 'n/a'} file=${currentFile}`);
}

async function main() {
  const [command = 'status', ...args] = process.argv.slice(2);
  if (command === 'set') return commandSet(args);
  if (command === 'validate') return commandValidate(args);
  if (command === 'render') return commandRender(args);
  if (command === 'play') return commandPlay(args);
  if (command === 'status') return commandStatus();
  if (command === 'skill') return commandSkill();
  if (command === 'devices') return commandDevices();
  if (command === 'open') return commandOpen();
  console.error(`Unknown command: ${command}`);
  process.exitCode = 1;
}

main().catch(error => {
  console.error(error.stack || String(error));
  process.exitCode = 1;
});

import { existsSync } from 'node:fs';
import { readFile, readdir, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { FAIL, PARTIAL, dashboardRows, floorOf, floorRows, pacedRows, passed, proofRows, spread, table, wholeHarness, wholeProof } from './playbackVerdicts.mjs';

const ROLES = ['floor', 'layers', 'layers-seeks', 'proof'];
const readJSON = async file => (existsSync(file) ? JSON.parse((await readFile(file, 'utf8')).replace(/^﻿/, '')) : null);

async function readLines(file) {
  if (!existsSync(file)) return null;
  const lines = (await readFile(file, 'utf8')).split(/\r?\n/).filter(line => line.trim());
  const records = [];
  let cut = false;
  for (const line of lines) {
    try { records.push(JSON.parse(line)); } catch { cut = true; }
  }
  return { records, frames: records, cut };
}

async function load(folder) {
  const run = await readJSON(path.join(folder, 'run.json'));
  if (!run) return null;
  const out = path.join(folder, 'harness');
  return { folder, run, report: await readJSON(path.join(out, 'report.json')), summary: await readJSON(path.join(out, 'summary.json')),
    frames: await readLines(path.join(out, 'frames.jsonl')) };
}

function whole(entry, acceptReconciliation = false) {
  return entry.run.kind === 'proof' ? wholeProof(entry.run, entry.summary, entry.frames) : wholeHarness(entry.run, entry.report, entry.frames, { acceptReconciliation });
}

const pct = value => (Number.isFinite(value) ? `${value.toFixed(0)}%` : '?');
function loadLine(run) {
  const { before, after } = run.load ?? {};
  const top = (before?.busy ?? []).slice(0, 5).map(entry => `${entry.name} ${entry.cpu.toFixed(0)}%`).join(', ');
  const memory = before?.free_memory_mb == null ? 'free memory not recorded' : `memory free ${before.free_memory_mb} → ${after?.free_memory_mb} MB, least ${run.memory?.least_free_mb ?? '?'} MB${run.memory_wait_seconds ? ` (held ${run.memory_wait_seconds} s for 8 GB)` : ''}`;
  return `CPU ${pct(before?.cpu_percent)} → ${pct(after?.cpu_percent)}${top ? ` (busiest before: ${top})` : ''}; ${memory}${before?.lock || after?.lock ? '; the timing lock was held' : ''}`;
}

async function judgeMap(phase, acceptReconciliation = false) {
  const names = (await readdir(phase, { withFileTypes: true })).filter(entry => entry.isDirectory()).map(entry => entry.name).sort();
  const runs = (await Promise.all(names.map(name => load(path.join(phase, name))))).filter(Boolean);
  if (!runs.length) throw new Error(`${phase} holds no runs`);
  const order = role => { const at = ROLES.indexOf(role); return at < 0 ? ROLES.length : at; };
  runs.sort((a, b) => String(a.run.page).localeCompare(String(b.run.page)) || (a.run.repeat ?? 0) - (b.run.repeat ?? 0) || order(a.run.role) - order(b.run.role));
  const rows = [];
  const floors = new Map();
  for (const entry of runs.filter(entry => entry.run.role === 'floor')) {
    const floor = floorOf({ ...entry, acceptReconciliation });
    const key = `${entry.run.page}\u0000${entry.run.repeat}`;
    if (floor.whole) floors.set(key, floor);
    rows.push(...floorRows({ page: entry.run.page, label: entry.run.label, floor, run: entry.run }).map(row => ({ ...row, repeat: entry.run.repeat, role: entry.run.role })));
  }
  for (const entry of runs.filter(entry => entry.run.role !== 'floor')) {
    const floor = floors.get(`${entry.run.page}\u0000${entry.run.repeat}`) ?? [...floors.entries()].find(([key]) => key.startsWith(`${entry.run.page}\u0000`))?.[1] ?? null;
    const made = entry.run.kind === 'proof'
      ? proofRows({ page: entry.run.page, label: entry.run.label, run: entry.run, summary: entry.summary, frames: entry.frames })
      : pacedRows({ page: entry.run.page, label: entry.run.label, run: entry.run, report: entry.report, frames: entry.frames, floor, acceptReconciliation });
    rows.push(...made.map(row => ({ ...row, repeat: entry.run.repeat, role: entry.run.role })));
  }
  for (const missing of (await readJSON(path.join(phase, 'phase.json')))?.unmeasurable ?? [])
    rows.push({ page: missing.page, run: missing.run, bar: missing.bar, measured: '–', limit: missing.limit ?? '', verdict: 'not measurable yet', worst: null, today: '', note: missing.why, values: {} });
  const lines = ['# Playback phase: the map', '', `${passed(rows) ? 'PASSED' : 'FAILED'}: ${rows.filter(row => row.verdict === FAIL).length} failing, ${rows.filter(row => row.verdict === PARTIAL).length} partial.`, '',
    '## Runs', '', '| Run | Kind | Exit | Whole | Cores | Load | Peak working set / private (runner) | Wall |', '|---|---|---|---|---|---|---|---|'];
  for (const entry of runs) {
    const verdict = whole(entry, acceptReconciliation);
    lines.push(`| ${entry.run.label} | ${entry.run.kind} | ${entry.run.exit ?? '–'}${entry.run.stopped ? ` (${entry.run.stopped})` : ''} | ${verdict.whole ? 'whole' : `**partial**: ${verdict.reasons.join('; ')}`} | ${entry.run.cores ?? 'not pinned'} | ${loadLine(entry.run)} | ${entry.run.peak ? `${entry.run.peak.working_set_mb} / ${entry.run.peak.private_mb} MB` : '–'} | ${entry.run.seconds?.toFixed(1) ?? '–'} s |`);
  }
  lines.push('', '## Verdicts', '', table(rows), '');
  const spreads = spread(rows);
  if (spreads.length) {
    lines.push('## Spread over repeats (least – greatest of each figure, whole runs only)', '', '| Page | Run | Bar | Runs | Figures | Verdicts |', '|---|---|---|---|---|---|');
    for (const entry of spreads) lines.push(`| ${entry.page} | ${entry.run} | ${entry.bar} | ${entry.runs} | ${Object.entries(entry.values).map(([name, [low, high]]) => `${name} ${low.toFixed(2)} – ${high.toFixed(2)}`).join('; ')} | ${entry.verdicts.join(', ')} |`);
    lines.push('');
  }
  const name = acceptReconciliation ? 'verdicts-accepting-reconciliation' : 'verdicts';
  if (acceptReconciliation) lines.splice(0, 1, '# Playback phase: the map, accepting the harness\'s CPU reconciliation exit (not results under the rule)');
  await writeFile(path.join(phase, `${name}.md`), lines.join('\n'));
  await writeFile(path.join(phase, `${name}.json`), JSON.stringify({ passed: passed(rows), rows, spread: spreads, runs: runs.map(entry => ({ ...entry.run, whole: whole(entry, acceptReconciliation) })) }, null, 1));
  console.log(`${passed(rows) ? 'passed' : 'failed'}: ${path.join(phase, `${name}.md`)}`);
  return passed(rows) ? 0 : 1;
}

async function verify(folder) {
  const entry = await load(folder);
  if (!entry) throw new Error(`${folder} holds no run.json`);
  const verdict = whole(entry);
  console.log(`${entry.run.label}: ${verdict.whole ? 'WHOLE' : `PARTIAL: ${verdict.reasons.join('; ')}`}`);
  return verdict.whole ? 0 : 1;
}

async function judgeDashboard(args) {
  const flag = name => { const at = args.indexOf(name); return at < 0 ? null : args[at + 1]; };
  const file = flag('--perf-log'), out = flag('--out');
  if (!file || !out) throw new Error('dashboard needs --perf-log <file> and --out <folder>');
  const log = await readLines(file);
  if (!log) throw new Error(`no perf log at ${file}`);
  const number = name => (flag(name) === null ? null : Number(flag(name)));
  const page = flag('--page') ?? path.basename(file);
  const { whole: complete, rows, follow } = dashboardRows(log, { page, refreshHz: number('--refresh-hz'), expect: { playingSeconds: number('--playing-seconds'), seeks: number('--seeks') } });
  const lines = ['# Playback phase: the dashboard', '', `${passed(rows) ? 'PASSED' : 'FAILED'}: ${rows.filter(row => row.verdict === FAIL).length} failing, ${rows.filter(row => row.verdict === PARTIAL).length} partial. `
    + `Perf log ${file}: ${complete.whole ? `whole, ${complete.played.toFixed(1)} s played` : `**partial**: ${complete.reasons.join('; ')}`}.`, '',
    '## Verdicts', '', table(rows), '', '## Followed, and fallen back', '', '| Widget | Followed | Fell back (why) |', '|---|---|---|'];
  for (const widget of follow.widgets) lines.push(`| ${widget.title} | ${widget.followed.join(', ') || '–'} | ${widget.none ? 'follows nothing: asks by request' : widget.fellBack.join('; ') || '–'} |`);
  await mkdir(out, { recursive: true });
  await writeFile(path.join(out, 'dashboard.md'), lines.join('\n') + '\n');
  await writeFile(path.join(out, 'dashboard.json'), JSON.stringify({ passed: passed(rows), whole: complete, rows, follow }, null, 1));
  console.log(`${passed(rows) ? 'passed' : 'failed'}: ${path.join(out, 'dashboard.md')}`);
  return passed(rows) ? 0 : 1;
}

const [command, ...rest] = process.argv.slice(2);
try {
  if (command === 'map') process.exitCode = await judgeMap(path.resolve(rest[0]), rest.includes('--accept-reconciliation'));
  else if (command === 'verify') process.exitCode = await verify(path.resolve(rest[0]));
  else if (command === 'dashboard') process.exitCode = await judgeDashboard(rest);
  else throw new Error('usage: playback.mjs map <phase> | verify <run> | dashboard --perf-log <file> --out <folder>');
} catch (error) {
  console.error(`The playback judge could not run: ${error.message}`);
  process.exitCode = 2;
}

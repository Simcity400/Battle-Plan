import { createHash } from 'node:crypto';
import { failureKey } from './coverage.mjs';
import { dashboardRows } from './playbackVerdicts.mjs';
import { quantiles } from './verdicts.mjs';

const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
export const scenarioId = value => createHash('sha256').update(JSON.stringify(canonical(value))).digest('hex');
export const scopeScenario = (recipe, context, visit) => ({ recipe, page: context.page, hour: context.hour, viewport: context.viewport, visit });
export const countKey = entry => scenarioId([entry.recording, entry.scenario, entry.interaction]);
export const sameVisualFixture = (a, b) => a?.name === b?.name && a?.bytes === b?.bytes && (!a?.sha256 || a.sha256 === b?.sha256);
export const GAPS = ['change map mode', 'open window', 'move map camera', 'native hover'];
export function familyOf(area, item, label) {
  if (area === 'startup') return 'open recording';
  if (['pages', 'pages-at'].includes(area)) return label === 'switch' ? 'open page' : 'scroll';
  if (area === 'seeks') return 'seek';
  if (area === 'controls') return item.startsWith('tooltips/') || item === 'header/play-pause' ? null
    : item === 'timeline/drag' || item === 'header/previous-next-hour' ? 'seek' : 'change setting';
  if (area === 'catalogue') return label === 'add' || ['delete', 'discard'].includes(label) ? 'add/remove widget' : 'change setting';
  return null;
}
export const timingReason = reason => /(?:\bms, over \d|^max \d+ ms, over|^frame p95 .* ms, over|^\d+ dropped frames in a row|^measured clock speed |^clock did not advance at |^frames with a new picture\/s \d|^worker presents\/s (?!.*none)|^React commits without verified structure or input \d|^showed a loading placeholder for \d+ ms)/i.test(reason);
export const functionalReasons = reasons => (reasons ?? []).filter(reason => !timingReason(reason));

export function measureCounts(run, results, records) {
  return results.filter(result => result.perf?.family).map(result => {
    const p = result.perf, log = (records ?? []).filter(record => record.session === result.session);
    const queries = log.filter(record => record.t === 'q' && !record.cache && record.cmd && record.at >= p.start && record.at < p.settled);
    let counts = { 'requests:total': queries.length };
    counts.rebinds = log.filter(r => r.t === 'rebind' && r.at >= p.start && r.at < p.settled).length;
    const breakdown = {};
    for (const q of queries) { const name = `requests:${q.product}`; counts[name] = (counts[name] ?? 0) + 1; const key = `${q.cmd}/${q.product}`; breakdown[key] = (breakdown[key] ?? 0) + 1; }
    let complete = p.complete && result.pass && Boolean(run.fixture?.sha256) && log.some(record => record.t === 'page' && record.rebinds)
      && log.some(record => record.t === 'stop' && record.at >= p.settled) && !log.some(record => record.t === 'lost');
    if (p.family === 'play') {
      const head = dashboardRows({ records: log, cut: false }, { window: { from: p.start, to: p.settled } }).rows.find(row => row.bar === 'requests for values at the head');
      counts = { 'requests:head': head?.values?.count ?? null };
      complete &&= Number.isInteger(counts['requests:head']);
    }
    const replySizes = queries.map(q => q.bytes).filter(Number.isFinite);
    const replyBytes = { observed: replySizes.reduce((sum, bytes) => sum + bytes, 0), measured: replySizes.length, sent: queries.length };
    return { recording: run.fixture?.sha256, scenario: p.scenario, interaction: `${result.session}|${failureKey(result)}`, family: p.family, phase: result.area,
      session: result.session, page: p.page, recipe: p.recipe, context: p.context, start: p.start, settled: p.settled, counts, breakdown, replyBytes, complete: Boolean(complete), reasons: [...functionalReasons(result.reasons), ...(p.reason ? [p.reason] : [])] };
  });
}
function index(entries) {
  const found = new Map();
  for (const entry of entries) { const key = countKey(entry); if (found.has(key)) throw new Error(`duplicate count key: ${entry.interaction}`); found.set(key, entry); }
  return found;
}
export function compareCounts(entries, baseline, expected = () => false) {
  if (baseline && baseline.version !== 1) throw new Error('unsupported perf baseline version');
  const accepted = index(baseline?.entries ?? []), measured = index(entries), rows = [];
  for (const [key, entry] of measured) {
    const before = accepted.get(key);
    if (!entry.complete) { rows.push({ entry, key, status: 'unmeasured', pass: false }); continue; }
    const core = Object.keys(entry.counts).filter(name => ['requests:total', 'requests:head', 'rebinds'].includes(name));
    for (const name of new Set([...core, ...Object.keys(before?.counts ?? entry.counts)])) {
      const value = entry.counts[name] ?? 0, prior = before?.counts[name], allowance = prior ? (prior.accepted === 0 ? 0 : Math.max(2, Math.ceil(prior.accepted * .2))) : null;
      const limit = prior ? prior.accepted + allowance : null;
      rows.push({ entry, key, name, value, before, allowance, limit, status: prior ? (value <= limit ? 'pass' : 'over') : 'unaccepted', pass: Boolean(prior && value <= limit) });
    }
  }
  for (const [key, entry] of accepted) if (!measured.has(key) && expected(entry)) rows.push({ entry, key, status: 'unmeasured', pass: false });
  return rows;
}
export function trend(run, results, records, entries) {
  const sessions = (run.sessions ?? []).map(s => {
    const frames = (records ?? []).filter(r => r.session === s.name.replaceAll(' ', '-') && r.t === 'frame');
    return { name: s.name, sessionCpuSeconds: s.sessionCpuSeconds, sessionPeakCommitBytes: s.sessionPeakCommitBytes, pictures: frames.reduce((n, r) => n + (r.pictures ?? 0), 0), commits: frames.reduce((n, r) => n + (r.commits ?? 0), 0),
      playingFrameMs: quantiles(frames.filter(r => r.playing).map(r => r.main)), playingIntervalMs: quantiles(frames.filter(r => r.playing).map(r => r.dt)) };
  });
  return { at: run.started, commit: run.commit, recording: run.fixture, sessions,
    interactions: results.map(({ area, item, label, ms, timing, placeholderMs, placeholderAt, trace }) => ({ area, item, label, ms, timing, placeholderMs, placeholderAt,
      ...(trace && { layout: { layouts: trace.layouts, dirtyObjects: trace.dirtyObjects, frames: trace.frames },
        mainThreadCpuMsPerFrame: Number.isFinite(trace.mainThreadCpuMs) && trace.frames > 0 ? trace.mainThreadCpuMs / trace.frames : null,
        traceElapsedMs: { main: trace.main, kinds: trace.kinds } }) })),
    counts: entries.map(({ interaction, scenario, counts, breakdown, replyBytes }) => ({ interaction, scenario, counts, breakdown, replyBytes })) };
}
export function countRow(row, run, file) {
  const { entry, name, value, before, allowance, limit } = row;
  const prefix = `${run.fixture?.name ?? 'recording'} [${entry.recording}] / ${entry.page ?? ''} / ${entry.interaction}: `;
  if (row.status === 'unmeasured') return `${prefix}unmeasured: ${entry.reasons?.join('; ') || 'action failed, scenario missing, or instrumentation incomplete'}. See ${file}.`;
  const accept = `If intended: suite.py accept --run "${file.replace(/[/\\]counts\.json$/, '')}" --counts ${row.key} --reason "…".`;
  if (row.status === 'unaccepted') return `${prefix}${name} ${value}; unaccepted. See ${file}. ${accept}`;
  return `${prefix}${name} ${value}; accepted ${before.counts[name].accepted}, allowance ${allowance}, limit ${limit} (+${value - before.counts[name].accepted}). Accepted ${before.acceptedAt}/${before.commit}: ${before.reason}. See ${file}. ${accept}`;
}

import { createHash } from 'node:crypto';
import { failureKey } from './coverage.mjs';
import { dashboardRows } from './playbackVerdicts.mjs';

const canonical = value => {
  if (value?.activeId && Array.isArray(value.layouts)) return canonical({ active: value.layouts.findIndex(e => e.id === value.activeId), layouts: value.layouts.map(e => e.template) });
  if (Array.isArray(value?.widgets) && Array.isArray(value.tracks) && Array.isArray(value.roles)) {
    const ids = new Map(['widgets', 'tracks', 'roles'].flatMap(group => value[group].map((e, i) => [e.id, `${group}:${i}`])));
    value = JSON.parse(JSON.stringify(value, (key, v) => typeof v === 'string' && ids.has(v) ? ids.get(v) : v));
  }
  return Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object'
    ? Object.fromEntries(Object.keys(value).sort().filter(key => key !== 'commit').map(key => [key, canonical(value[key])])) : value;
};
export const scenarioId = value => createHash('sha256').update(JSON.stringify(canonical(value))).digest('hex');
export const countKey = entry => scenarioId([entry.recording, entry.scenario, entry.interaction]);
export const GAPS = ['change map mode', 'open window', 'move map camera', 'native hover'];
export function familyOf(area, item, label) {
  if (area === 'startup') return 'open recording';
  if (['pages', 'pages-at'].includes(area)) return label === 'switch' ? 'open page' : 'scroll';
  if (area === 'seeks') return 'seek';
  if (area === 'controls') return item.startsWith('tooltips/') ? null : item === 'header/play-pause' ? (label === 'pause' ? 'pause' : null)
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
    if (log.some(r => r.t === 'page' && r.rebinds)) counts.rebinds = log.filter(r => r.t === 'rebind' && r.at >= p.start && r.at < p.settled).length;
    for (const q of queries) for (const name of [`requests:${q.product}`, `commands:${q.cmd}`]) counts[name] = (counts[name] ?? 0) + 1;
    let complete = p.complete && result.pass && Boolean(run.fixture?.sha256) && log.some(record => record.t === 'page' && record.rebinds)
      && log.some(record => record.t === 'stop' && record.at >= p.settled) && !log.some(record => record.t === 'lost');
    if (p.family === 'play') {
      const head = dashboardRows({ records: log, cut: false }, { window: { from: p.start, to: p.settled } }).rows.find(row => row.bar === 'requests for values at the head');
      counts = { 'requests:head': head?.values?.count ?? null };
      complete &&= Number.isInteger(counts['requests:head']);
    }
    return { recording: run.fixture?.sha256, scenario: p.scenario, interaction: `${result.session}|${failureKey(result)}`, family: p.family, phase: result.area,
      session: result.session, page: p.page, context: p.context, start: p.start, settled: p.settled, counts, complete: Boolean(complete), reasons: functionalReasons(result.reasons) };
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
    for (const name of new Set([...Object.keys(entry.counts), ...Object.keys(before?.counts ?? {})])) {
      const value = entry.counts[name] ?? 0, prior = before?.counts[name], allowance = prior ? (prior.accepted === 0 ? 0 : Math.max(2, Math.ceil(prior.accepted * .2))) : null;
      const limit = prior ? prior.accepted + allowance : null;
      rows.push({ entry, key, name, value, before, allowance, limit, status: prior ? (value <= limit ? 'pass' : 'over') : 'unaccepted', pass: Boolean(prior && value <= limit) });
    }
  }
  for (const [key, entry] of accepted) if (!measured.has(key) && expected(entry)) rows.push({ entry, key, status: 'unmeasured', pass: false });
  return rows;
}
export function countRow(row, run, file) {
  const { entry, name, value, before, allowance, limit } = row;
  const prefix = `${run.fixture?.name ?? 'recording'} [${entry.recording}] / ${entry.page ?? ''} / ${entry.interaction}: `;
  if (row.status === 'unmeasured') return `${prefix}unmeasured: ${entry.reasons?.join('; ') || 'action failed, scenario missing, or instrumentation incomplete'}. See ${file}.`;
  const accept = `If intended: suite.py accept --run "${file.replace(/[/\\]counts\.json$/, '')}" --counts ${row.key} --reason "…".`;
  if (row.status === 'unaccepted') return `${prefix}${name} ${value}; unaccepted. See ${file}. ${accept}`;
  return `${prefix}${name} ${value}; accepted ${before.counts[name].accepted}, allowance ${allowance}, limit ${limit} (+${value - before.counts[name].accepted}). Accepted ${before.acceptedAt}/${before.commit}: ${before.reason}. See ${file}. ${accept}`;
}

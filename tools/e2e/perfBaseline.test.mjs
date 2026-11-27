import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { compareCounts, scenarioId, scopeScenario, measureCounts, countRow, functionalReasons, familyOf, sameVisualFixture, trend } from './perfBaseline.mjs';
import { Session } from './session.mjs';

const run = { fixture: { name: 'fixture', sha256: 'sha256:fixture' }, commit: 'rev', started: 'today' };
const entry = { recording: run.fixture.sha256, scenario: 'scenario', interaction: 'seeks|Army|early warm', family: 'seek', phase: 'seeks', session: 'seeks', counts: { 'requests:total': 8 }, complete: true };
const accepted = (n = 8) => ({ version: 1, entries: [{ ...entry, counts: { 'requests:total': { accepted: n, tolerance: { absolute: n ? 2 : 0, relative: n ? .2 : 0 } } }, acceptedAt: 'date', commit: 'base', reason: 'reviewed' }] });
test('one-sided counts allow slack, reject duplication, and preserve zero invariants', () => {
  assert.equal(compareCounts([{ ...entry, counts: { 'requests:total': 10 } }], accepted())[0].pass, true);
  const over = compareCounts([{ ...entry, counts: { 'requests:total': 14 } }], accepted())[0];
  assert.equal(over.pass, false);
  assert.match(countRow(over, run, 'run/counts.json'), /14; accepted 8, allowance 2, limit 10 \(\+6\)/);
  assert.match(countRow(over, run, 'run/counts.json'), /Accepted date\/base: reviewed/);
  assert.equal(compareCounts([{ ...entry, counts: { 'requests:total': 1 } }], accepted(0))[0].pass, false);
});
test('new, missing, and incomplete entries fail without pretending zero work', () => {
  assert.equal(compareCounts([entry], null)[0].status, 'unaccepted');
  assert.equal(compareCounts([], accepted(), () => true)[0].status, 'unmeasured');
  assert.equal(compareCounts([{ ...entry, complete: false }], accepted())[0].status, 'unmeasured');
  assert.equal(compareCounts([], accepted(), () => false).length, 0);
  assert.throws(() => compareCounts([entry, entry], null), /duplicate/);
  assert.throws(() => compareCounts([entry], { ...accepted(), entries: [...accepted().entries, ...accepted().entries] }), /duplicate/);
});
test('scenario is pinned by recipe and starting state, independent of output labels and random saved ids', () => {
  const recipe = { branchIndex: 0, tier: 'full', cases: ['widget/settings'], settings: { options: 'all' } };
  const context = { page: 'Example', hour: 5, viewport: [800, 600, 1], settings: { 'page-random1': 'Acceptance 111' }, controls: ['old label'], commit: 'old' };
  const scope = scopeScenario(recipe, context, 'warm');
  assert.equal(scenarioId(scope), scenarioId(scopeScenario(recipe, { ...context, settings: { 'page-random2': 'Acceptance 222' }, controls: ['new label'], commit: 'new' }, 'warm')));
  assert.equal(scenarioId(scope), scenarioId({ visit: 'warm', viewport: context.viewport, hour: 5, page: 'Example', recipe }));
  for (const key of ['recipe', 'hour', 'viewport', 'visit', 'page']) assert.notEqual(scenarioId(scope), scenarioId({ ...scope, [key]: 'different' }));
  assert.notEqual(scenarioId(scope), scenarioId(scopeScenario({ ...recipe, branchIndex: 1 }, context, 'warm')));
});
test('new product names stay in the breakdown and do not bypass the total request allowance', () => {
  const baseline = accepted(23);
  const measured = { ...entry, counts: { 'requests:total': 25, 'requests:new-product': 2 } };
  assert.equal(compareCounts([measured], baseline).every(row => row.pass), true);
  baseline.entries[0].counts['requests:stable-product'] = { accepted: 4 };
  assert.equal(compareCounts([{ ...measured, counts: { ...measured.counts, 'requests:stable-product': 7 } }], baseline).some(row => row.name === 'requests:stable-product' && !row.pass), true);
});
test('visual acceptances made before recording hashes stay valid for the same fixture', () => {
  assert.equal(sameVisualFixture({ name: 'fixture', bytes: 123 }, { name: 'fixture', bytes: 123, sha256: 'hash' }), true);
  assert.equal(sameVisualFixture({ name: 'fixture', bytes: 123 }, { name: 'fixture', bytes: 124 }), false);
  assert.equal(sameVisualFixture({ name: 'fixture', bytes: 123, sha256: 'first' }, { name: 'fixture', bytes: 123, sha256: 'second' }), false);
});
test('history contains trend totals rather than per-frame logs or trace stacks', () => {
  const frames = Array.from({ length: 100 }, () => ({ t: 'frame', session: 'example', pictures: 2, commits: 3 }));
  const history = trend({ ...run, sessions: [{ name: 'example', sessionPeakCommitBytes: 99 }] }, [{ area: 'trace', trace: { layouts: 3, dirtyObjects: 4, frames: 5, roots: ['not trend data'] } }], frames, []);
  assert.equal(history.sessions[0].sessionPeakCommitBytes, 99);
  assert.equal(history.sessions[0].pictures, 200);
  assert.equal(history.sessions[0].commits, 300);
  assert.deepEqual(history.interactions[0].layout, { layouts: 3, dirtyObjects: 4, frames: 5 });
  assert.equal('frames' in history, false);
  assert.equal(JSON.stringify(history).includes('not trend data'), false);
});
test('measurement counts completed actual sends including errors, excludes hits and joins', () => {
  const result = { area: 'seeks', item: 'Army', label: 'early warm', session: 'seeks', pass: true, reasons: [], perf: { start: 100, settled: 200, complete: true, scenario: 'scenario', family: 'seek' } };
  const records = [{ t: 'page', session: 'seeks', at: 0, rebinds: true }, { t: 'stop', session: 'seeks', at: 250 }, ...['ok', 'cancelled', 'error'].map(outcome => ({ t: 'q', at: 150, session: 'seeks', cmd: 'query', product: 'countries', outcome })), { t: 'q', at: 150, session: 'seeks', cache: 'hit' }, { t: 'q', at: 160, session: 'seeks', cache: 'joined' }];
  const measured = measureCounts(run, [result], records)[0];
  assert.equal(measured.counts['requests:total'], 3);
  assert.equal(measured.counts['requests:countries'], 3);
  assert.equal(measureCounts(run, [result], [...records, { t: 'lost', session: 'seeks', count: 1 }])[0].complete, false);
  assert.equal(measureCounts(run, [result], [])[0].complete, false);
});
test('transient placeholders are timing trends; content still missing at settlement fails by name', () => {
  assert.deepEqual(functionalReasons(['showed a loading placeholder for 123 ms (Widget)']), []);
  assert.deepEqual(functionalReasons(['Widget: still loading after 6 s']), ['Widget: still loading after 6 s']);
  assert.deepEqual(functionalReasons(['never finished: expected content']), ['never finished: expected content']);
});
test('unwatch preserves a still-visible placeholder as a fatal named reason', async () => {
  const session = new Session({ evaluate: async () => ({ ms: 123, where: ['Example'], still: ['Example'] }) });
  const reasons = await session.unwatch();
  assert.match(reasons[0], /123 ms/);
  assert.deepEqual(functionalReasons(reasons), ['Example: still showing a loading placeholder at settlement']);
});
test('seven families gate actual work; pause stays unmeasured without a deterministic clock', () => {
  const families = [['startup', 'open', 'open'], ['pages', 'cold/Example', 'switch'], ['pages', 'cold/Example', 'scroll 1'],
    ['seeks', 'Example', 'early'], ['controls', 'header/play-pause', 'pause'], ['controls', 'widget/settings', 'choose'], ['catalogue', 'view/Example', 'add']]
    .map(([area, item, label]) => familyOf(area, item, label));
  const result = { area: 'playback', item: 'Example/24', session: 'play', pass: true, perf: { family: 'play', start: 100, settled: 200, complete: true, scenario: 'play' } };
  const records = [{ t: 'page', at: 0, rebinds: true }, { t: 'stop', at: 250 }, { t: 'tick', at: 110, playing: true, hour: 1 },
    { t: 'tick', at: 190, playing: true, hour: 2 }, { t: 'frame', at: 150, playing: true, dt: 16 },
    { t: 'q', at: 170, cmd: 'query', product: 'series-tiles', from: 50, head: 1 }].map(r => ({ ...r, session: 'play' }));
  const measured = measureCounts(run, [result], records)[0];
  assert.equal(measured.complete, true);
  assert.deepEqual(measured.counts, { 'requests:head': 0 });
  assert.equal(familyOf('controls', 'header/play-pause', 'pause'), null);
  assert.equal(new Set([...families, measured.family].filter(Boolean)).size, 7);
});
test('the report makes transient placeholders informational, keeps their duration, and rejects missing content', async t => {
  const folder = await mkdtemp(path.join(tmpdir(), 'bp-count-report-'));
  t.after(() => rm(folder, { recursive: true }));
  const work = path.join(folder, 'work'), out = path.join(folder, 'run');
  await mkdir(work); await mkdir(out);
  const metadata = { ...run, requested: ['controls'], sessions: [] };
  await writeFile(path.join(out, 'run.json'), JSON.stringify(metadata));
  const result = { area: 'startup', item: 'open', label: 'open', session: 'test', pass: false, ms: 150, reasons: ['showed a loading placeholder for 123 ms (spinner)'],
    perf: { family: 'open recording', start: 100, settled: 200, complete: true, scenario: 's' }, placeholderMs: 123, placeholderAt: ['spinner'] };
  const sweep = { meta: { session: 'test', phases: ['controls'], pages: [], seeks: [], speeds: [], finished: 'today' }, results: [result], console: [] };
  const log = [{ t: 'page', at: 0, rebinds: true }, { t: 'stop', at: 250 }];
  await writeFile(path.join(work, 'perf-test.jsonl'), log.map(r => JSON.stringify(r)).join('\n'));
  const report = () => spawnSync(process.execPath, [new URL('./report.mjs', import.meta.url).pathname.replace(/^\/(\w:)/, '$1'), '--run', path.join(out, 'run.json'), '--sweeps', out, '--perf', work, '--out', out, '--perf-baseline', path.join(folder, 'perf-baseline.json')], { encoding: 'utf8' });
  await writeFile(path.join(out, 'sweep-test.json'), JSON.stringify(sweep));
  const first = report();
  assert.equal(first.status, 1, first.stderr);
  const counts = JSON.parse(await readFile(path.join(out, 'counts.json'), 'utf8'));
  assert.equal(counts.entries[0].complete, true);
  assert.match(await readFile(path.join(out, 'e2e.md'), 'utf8'), /unaccepted/);
  assert.match(await readFile(path.join(out, 'e2e.md'), 'utf8'), /7 available.*Pause is unmeasured:.*deterministic playback clock/);
  assert.equal(JSON.parse((await readFile(path.join(folder, 'perf-history.jsonl'), 'utf8')).trim()).interactions[0].placeholderMs, 123);
  await writeFile(path.join(folder, 'perf-baseline.json'), JSON.stringify({ version: 1, entries: counts.entries.map(e => ({ ...e, counts: Object.fromEntries(Object.keys(e.counts).map(name => [name, { accepted: 0 }])), reason: 'reviewed' })) }));
  assert.equal(report().status, 0);
  result.reasons = ['Example widget: still loading after 6 s']; result.perf.complete = false;
  await writeFile(path.join(out, 'sweep-test.json'), JSON.stringify(sweep));
  assert.equal(report().status, 1);
  assert.match(await readFile(path.join(out, 'e2e.md'), 'utf8'), /Example widget: still loading after 6 s/);
});

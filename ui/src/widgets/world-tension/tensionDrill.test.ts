import assert from 'node:assert/strict';
import test from 'node:test';
import { registerWidgets } from '../../dashboard/definitions.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options.ts';
import type { TensionReply } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition.ts';
import { compactStack } from '../../charts/stack.ts';
import { runAt } from '../../charts/series.ts';
import { orderedDrillParts } from '../../charts/drill/parts.ts';
import { tensionModel, tensionPlot } from './tensionData.ts';
import { pinTensionDrill, tensionCauseAt, tensionDrillAt, tensionDrillHistory, tensionEventKeys, tensionLookAt, tensionShareText, tensionVisibleParts } from './tensionDrill.ts';
import { createElement } from 'react';
import type { ComponentProps } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type TensionDrill from './TensionDrillView';
import type DrillParts from '../../charts/drill/DrillParts';
import type { Select } from '../../components/Select';

const reply = (): TensionReply => ({ branchId: 'branch', tensionScale: 1000, contributionScale: 100000, ceilingRaw: '10000000', warnings: [],
  reads: [{ firstHour: 100, lastHour: 123, startsSegment: true, tensionRaw: '100000', sumRaw: '13350000', unnamedRaw: '50000' },
    { firstHour: 150, lastHour: 150, startsSegment: true, tensionRaw: '0', sumRaw: '-300000', unnamedRaw: '0' }],
  countries: [{ tag: 'mod country', slot: 1, runs: [{ firstHour: 100, lastHour: 123, valueRaw: '13500000', startsSegment: true }] },
    { tag: 'same tag', slot: 2, runs: [{ firstHour: 100, lastHour: 123, valueRaw: '-200000', startsSegment: true }] }],
  events: [{ hour: 99, tag: 'mod country', reason: 'Native cause text', firstRead: 100, lastRead: 123,
    added: [{ hour: 100, addedRaw: '20000000' }, { hour: 120, addedRaw: '19000000' }] }],
});

test('tension drill uses plain cause controls while retaining signed contributions and totals', async () => {
  const state = { parts: undefined as ComponentProps<typeof DrillParts> | undefined, select: undefined as ComponentProps<typeof Select> | undefined,
    requests: [] as unknown[], style: undefined as unknown };
  const host = globalThis as typeof globalThis & { __tensionPresentation?: typeof state }; host.__tensionPresentation = state;
  const replacements: Record<string, string> = {
    '/dashboard/preparedSeries.ts': `export const usePreparedWindow = (_client,_source,product,params) => {
      globalThis.__tensionPresentation.requests.push({product,params}); return { geometryReady: false,owner: {},pickOwner: {},onSummaryView() {},onNavigateHour() {} }; };`,
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = (_client,_track,wanted) => new Map(wanted.map(row => [nameKey(row.hour,row.tag),globalThis.__tensionPresentation.style]));',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryFlagsAt = () => new Map(); export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/useCountriesAt.ts': 'export const STYLES = {};',
    '/dashboard/countryLooks.ts': 'export const lookHour = (_branch, hour) => ({ hour });',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000; export const useChartTokens = () => ({ muted: "#888888", surface: "#000000", countryColours: "game" });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    '/components/Select.tsx': 'export const Select = props => { globalThis.__tensionPresentation.select = props; return props.label; };',
    '/charts/drill/DrillParts.tsx': 'export default props => { globalThis.__tensionPresentation.parts = props; return props.totalRow.label; };',
    '/charts/TimeChart.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'tension-drill-presentation', enforce: 'pre', transform(_code, id) {
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Component } = await vite.ssrLoadModule('/ui/src/widgets/world-tension/TensionDrillView.tsx');
    const track = { id: 'main', source: { id: 'source', revision: '1' }, branch: { id: 'branch', firstHour: 99, lastHour: 150, gaps: [] } } as unknown as BoundTrack;
    const picks: (string | null)[] = [];
    const props = { model: tensionModel(reply()), runtime: { client: {}, tracks: [track], clock: { getSnapshot: () => ({ hoursPerSecond: 0 }) } }, track,
      active: true, options: { playback: 'cursor' }, extent: [99, 150], playhead: null, hour: 110, selectedCause: null,
      onCause: (key: string | null) => picks.push(key), onPick() {} } as unknown as ComponentProps<typeof TensionDrill>;
    const render = () => renderToStaticMarkup(createElement(Component, props));
    assert.match(render(), /Total contributions/);
    assert.deepEqual(state.requests[0],{product:'world-tension',params:{branchId:'branch',share:false}});
    assert.equal(state.select!.label, 'Cause');
    assert.equal(state.select!.options[0].label, 'Choose a cause');
    const [key] = tensionEventKeys(reply().events);
    assert.equal(state.select!.options[1].value, key);
    state.select!.onChange(key); state.select!.onChange(''); assert.deepEqual(picks, [key, null]);
    assert.equal(state.parts!.unattributed?.label, 'Other');
    assert.deepEqual(state.parts!.parts.map(part => part.raw), [13500000n, -200000n]);
    assert.ok(state.parts!.parts.every(part => part.label === 'Loading name…'));
    state.style = null; render();
    assert.ok(state.parts!.parts.every(part => part.label === 'Country name unavailable'));
    state.style = { name: null }; render();
    assert.ok(state.parts!.parts.every(part => part.label === 'Country name unavailable'));
    state.style = { name: '' }; render();
    assert.ok(state.parts!.parts.every(part => part.label === ''));
    state.style = undefined; props.active = false; render();
    assert.ok(state.parts!.parts.every(part => part.label === 'Country name unavailable'));
    props.active = true;
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    for (const error of [new Error('Synthetic tension style decoder failure'), new RefusedError('Synthetic tension names were not recorded')]) {
      state.style = error;
      assert.match(render(), new RegExp('data-detail="' + error.message + '"'));
      assert.ok(state.parts!.parts.every(part => part.label === (error instanceof RefusedError ? error.message : 'Couldn’t load country name.')));
      assert.ok(state.parts!.parts.every(part => part.detail === error.message));
    }
    props.selectedCause = key;
    const selected = render();
    assert.match(selected, /Final contribution/);
    assert.doesNotMatch(selected, /Selected recorded cause|Read |read once a day|Current contribution: not known/);
  } finally { delete host.__tensionPresentation; await vite.close(); }
});

test('full tension drill keeps signed native aggregates, the exact unidentified remainder and independent world total', () => {
  const model = tensionModel(reply());
  const at = tensionDrillAt(model, 110)!;
  assert.deepEqual(at.parts.map(part => part.raw), ['13500000', '-200000']);
  assert.equal(at.remainder, '50000');
  assert.equal(at.read.sumRaw, '13350000');
  assert.equal(at.read.tensionRaw, '100000');
  const stack = tensionPlot(model, at.parts.map(part => part.key), false);
  assert.deepEqual(stack.edges[0]?.[1], { low: -2, high: 0 });
  assert.equal(stack.edges[0]?.at(-1)?.high, 100);
  assert.equal(tensionDrillAt(model, 124), null);
  assert.equal(tensionDrillAt(model, 150)?.remainder, '-300000');
});

test('all 5000 native country slots retain separate drill rows even when their tags coincide', () => {
  const data = reply();
  data.countries = Array.from({ length: 5000 }, (_, slot) => ({ tag: 'shared tag', slot,
    runs: [{ firstHour: 100, lastHour: 123, valueRaw: '1', startsSegment: true }] }));
  data.reads[0].sumRaw = '5000';
  const at = tensionDrillAt(tensionModel(data), 110)!;
  assert.equal(at.parts.length, 5000);
  assert.equal(new Set(at.parts.map(part => part.key)).size, 5000);
  assert.equal(at.remainder, '0');
});

test('a selected cause never backfills a final amount or treats it as the current aggregate', () => {
  const data = reply();
  const event = data.events[0];
  assert.equal(tensionCauseAt(event, 99), null);
  assert.deepEqual(tensionCauseAt(event, 110), { hour: 100, addedRaw: '20000000' });
  assert.deepEqual(tensionCauseAt(event, 130), { hour: 120, addedRaw: '19000000' });
  assert.notEqual(tensionCauseAt(event, 110)?.addedRaw, tensionDrillAt(tensionModel(data), 110)!.parts[0].raw);
});

test('a chosen cause keeps its event when preparation adds an earlier one', () => {
  const event = (hour: number, firstRead: number, reason: string, tag: string | null) => ({ hour, tag, reason, firstRead, lastRead: firstRead,
    added: [{ hour: firstRead, addedRaw: '1' }] });
  const before = tensionEventKeys([event(200, 210, 'Same text', null), event(200, 210, 'Same text', 'BBB'), event(200, 210, 'Other text', 'CCC')]);
  // A source first read later but dated earlier sorts first; a source named later keeps its identity.
  const after = tensionEventKeys([event(150, 300, 'Same text', 'DDD'), event(200, 210, 'Same text', 'AAA'), event(200, 210, 'Same text', 'BBB'),
    event(200, 210, 'Other text', 'CCC')]);
  assert.deepEqual(after.slice(1), before);
  assert.equal(new Set(after).size, after.length);
});

test('tension drill is an ordinary saved view with inline defaults, full country choices and a held-day pin', () => {
  registerWidgets([definition]);
  assert.equal(resolveOptions(OPTIONS, undefined).drill, 'inline');
  assert.equal(resolveOptions(OPTIONS, undefined).show, 'main');
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const source = { id: 'source', kind: definition.kind, configVersion: 1, title: 'World tension', campaign: 'tracks', trackIds: ['track'],
    countries: { kind: 'all' }, time: { mode: 'follow' }, contentScale: 125, options: { drill: 'popout', values: 'share' } } satisfies WidgetSpec;
  const track = { id: 'track', branch: { firstHour: 100 } } as Pick<BoundTrack, 'id' | 'branch'>;
  const held = pinTensionDrill(source, track, 120.9, false);
  assert.deepEqual(held.time, { mode: 'pin', windows: { track: { firstOffset: 20, lastOffset: 20 } } });
  assert.equal(resolveOptions(OPTIONS, held.options).show, 'breakdown');
  assert.deepEqual(held.countries, source.countries);
  assert.deepEqual(pinTensionDrill(source, track, 120, true).time, source.time);
  assert.deepEqual(pinTensionDrill(held, track, 130, true).time, { mode: 'follow' });
});

test('signed shares and wide native ordering never depend on rounded plotting numbers', () => {
  assert.equal(tensionShareText('-200000', '13350000'), '\u22121.50%');
  assert.equal(tensionShareText('-1', '-2'), '\u221250.00%');
  assert.equal(tensionShareText('1', null), 'Not known');
  assert.equal(tensionShareText('1', '0'), 'Not known');
  const rows = orderedDrillParts([
    { id: 'small', label: 'Small', value: Number(9007199254740992n), raw: 9007199254740992n },
    { id: 'large', label: 'Large', value: Number(9007199254740993n), raw: 9007199254740993n },
    { id: 'correction', label: 'Correction', value: Number(-9007199254740994n), raw: -9007199254740994n },
  ]);
  assert.deepEqual(rows.map(row => row.id), ['correction', 'large', 'small']);
  assert.equal(tensionShareText('-9007199254740994', '9007199254740993'), '\u2212100.00%');
});

test('ten years of unchanged 5000-country daily reads retain only native changes and all countries', () => {
  const data = reply();
  const days = 3650;
  data.reads = Array.from({ length: days }, (_, day) => ({ firstHour: 100 + day * 24, lastHour: 123 + day * 24,
    startsSegment: day === 0, tensionRaw: '5000', sumRaw: '500000', unnamedRaw: '0' }));
  data.countries = Array.from({ length: 5000 }, (_, slot) => ({ tag: `opaque ${slot}`, slot,
    runs: [{ firstHour: 100, lastHour: data.reads.at(-1)!.lastHour, valueRaw: '100', startsSegment: true }] }));
  const model = tensionModel(data);
  const history = tensionDrillHistory(model, [...model.raw.keys()]);
  assert.equal(history.bands.length, 5001);
  assert.equal(history.bands.reduce((sum, band) => sum + band.length, 0), 5001);
  assert.equal(history.world.length, 1);
  assert.equal(compactStack(history.bands).hours.length, 3);
  assert.equal(runAt(history.bands[4999], data.reads.at(-1)!.lastHour)?.value, 0.001);
  assert.deepEqual(tensionVisibleParts(tensionDrillAt(model, 110)!.parts, ['opaque 4999\u00004999']).map(part => part.key), ['opaque 4999\u00004999']);
});

test('compressed full history preserves signed residuals, independent world totals, zeros and gaps', () => {
  const model = tensionModel(reply());
  const history = tensionDrillHistory(model, [...model.raw.keys()]);
  assert.deepEqual(history.bands.map(band => runAt(band, 110)?.value), [135, -2, 0.5]);
  assert.equal(runAt(history.world, 110)?.value, 100);
  assert.ok(history.bands.every(band => runAt(band, 124) == null));
  assert.deepEqual(history.bands.map(band => runAt(band, 150)?.value), [0, 0, -3]);
  assert.ok(history.bands.every(band => runAt(band, 150)?.startsSegment));
});

test('country looks distinguish native slots sharing a reused tag and refuse an unproved holder', () => {
  const data = reply();
  data.countries = [
    { tag: 'OLD', slot: 1, runs: [{ firstHour: 100, lastHour: 123, valueRaw: '100', startsSegment: true }],
      identities: [{ firstHour: 100, lastHour: 105, slot: 1, countryId: 11 }] },
    { tag: 'OLD', slot: 2, runs: [{ firstHour: 110, lastHour: 123, valueRaw: '200', startsSegment: false }],
      identities: [{ firstHour: 110, lastHour: 123, slot: 2, countryId: 22 }] },
  ];
  const branch = { firstHour: 100, lastHour: 150, gaps: [] };
  const model = tensionModel(data);
  assert.equal(tensionLookAt(model, 'OLD\u00001', 120, branch)?.hour, 100);
  assert.equal(tensionLookAt(model, 'OLD\u00002', 120, branch)?.hour, 110);
  data.countries[0].identities!.push({ firstHour: 115, lastHour: 123, slot: 1, countryId: 99 });
  assert.equal(tensionLookAt(tensionModel(data), 'OLD\u00001', 120, branch), null);
  delete data.countries[0].identities;
  assert.equal(tensionLookAt(tensionModel(data), 'OLD\u00001', 120, branch), null);
});

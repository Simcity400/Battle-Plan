import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentProps, ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { NavalLossesReply } from '../../generated/dashboard';
import { engineHour, gameDate } from '../../dashboard/calendar.ts';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState.ts';
import { plotFrame } from '../../charts/frame.ts';
import { ChartPicks } from '../../charts/pick.ts';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type CountryPicker from '../../components/CountryPicker';
import type DrillHost from '../../charts/drill/DrillHost';
import { reducePartsRaceDrill } from '../../charts/race/partsRaceDrill.ts';
import type { PartsRaceDrillState } from '../../charts/race/partsRaceDrill';
import { pickLines } from '../../charts/linePick.ts';
import { navalModel, navalReadHour, navalValue } from './navalLossesData.ts';
import { runAt } from '../../charts/series.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { definition } from './definition.ts';
import { navalWindowFixture } from './navalWindowFixture.ts';

test('known zero naval rows render inside a wrapper that fills the widget height', async () => {
  const reply: NavalLossesReply = { branchId: '1', available: true, firstRead: 100, shipFirstRead: 100,
    convoyFirstRead: 100, shipLastRead: 100, convoyLastRead: 100, shipReads: [100], convoyReads: [100], warnings: [],
    series: ['mod country A', 'mod country B'].map(tag => ({ tag, membership: null,
      segments: [{ firstHour: 100, lastHour: 105 }],
      runs: [{ firstHour: 100, lastHour: 105, startsSegment: true, ships: 0, convoys: 0 }] })) };
  // Keep the widget, drill host and race view; replace transport and country artwork.
  const replacements: Record<string, string> = {
    '/dashboard/productWindows.ts': navalWindowFixture,
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(reply)} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackMotion = () => 1;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => tag,
      color: () => [50, 100, 150], flag: () => undefined, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456', '#654321'],
      muted: '#888888', surface: '#000000', countryColours: 'raw', fontSize: { body: 12 } });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'naval-zero-fixture', enforce: 'pre', transform: (_source, id) => {
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx');
    const widget = { id: 'naval', kind: 'naval-losses', trackIds: ['track'], countries: definition.defaultCountries,
      time: { mode: 'campaign' }, options: {} };
    const track = { id: 'track', source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
      branch: { id: '1', firstHour: 100, lastHour: 105 } };
    const runtime = { tracks: [track], client: {}, clock: { getHour: () => 105 } };
    const html = renderToStaticMarkup(createElement(Widget, { widget, runtime, active: true, onDiagnostics() {} }));
    assert.match(html, /^<div style="[^"]*height:100%;/);
    assert.match(html, /class="drill-host"/);
    assert.match(html, /class="race-chart"/);
    // A top N of known zeros ranks no one (by design): it says so, inside the same filling wrapper.
    assert.match(html, /No known naval losses yet/);
    assert.match(html, /Since Hour 100/);
  } finally { await vite.close(); }
});

test('a February race pick retains its displayed zero month while Jump uses the January read', async () => {
  const january = engineHour({ year: 1936, month: 1, day: 31, hour: 24 })!, february = january + 1;
  assert.equal(gameDate(january)?.month, 1);
  assert.equal(gameDate(february)?.month, 2);
  const reply: NavalLossesReply = { branchId: '1', available: true, firstRead: january, shipFirstRead: january,
    convoyFirstRead: january, shipLastRead: january, convoyLastRead: january, shipReads: [january], convoyReads: [january], warnings: [],
    series: [{ tag: 'mod country A', membership: null, segments: [{ firstHour: january, lastHour: february }], runs: [
      { firstHour: january, lastHour: january, startsSegment: true, ships: 4, convoys: 2 },
      { firstHour: february, lastHour: february, startsSegment: false, ships: 0, convoys: 0 },
    ] }] };
  const picks: [number, string | undefined][] = [], jumps: number[] = [];
  const replacements: Record<string, string> = {
    '/dashboard/productWindows.ts': navalWindowFixture,
    '/charts/race/RaceChart.tsx': `export default function Race(p) {
      (p.onPick ?? p.onSelect)?.('mod country A'); p.onContextMenu?.('mod country A', 0, 0); return null;
    }`,
    '/charts/jump.ts': 'export const jumpItem = (runtime, _track, hour) => ({ run: () => runtime.requestSeek(hour) });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 1;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => tag,
      color: () => [50, 100, 150], flag: () => undefined, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'],
      muted: '#888888', surface: '#000000', countryColours: 'raw' }); export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open: (_x, _y, items) => items[0].run(), menu: null });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'naval-month-pick', enforce: 'pre', transform: (_source, id) =>
      Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1] }] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/charts/race/CountryRaceView.tsx');
    const track = { id: 'track', source: { id: 'synthetic' }, branch: { id: '1', firstHour: january, lastHour: february } };
    renderToStaticMarkup(createElement(View, { runtime: { client: {}, clock: {}, tracks: [track], requestSeek: (hour: number) => jumps.push(hour) },
      track, active: true, options: {}, view: 'race', extent: [january, february], playhead: null, moment: february, slots: 1,
      ranked: [{ tag: 'mod country A', value: 0, label: '0', stretch: january }], lines: [], label: 'Naval losses', empty: '',
      formatValue: String, tooltip: () => null, countryLookHour: (_tag: string | null, hour: number) => navalReadHour(reply, hour, 'both'),
      onPick: (hour: number, tag?: string) => picks.push([hour, tag]) }));
    assert.deepEqual(picks, [[february, 'mod country A']]);
    assert.deepEqual(jumps, [january]);
    assert.equal(drillHour(reduceDrill(CLOSED, { type: 'open', breakdown: picks[0][1]!, hour: picks[0][0],
      playing: false, playhead: null }), null), february);
    const model = navalModel(reply);
    assert.equal(navalValue(runAt(model.series.get(picks[0][1]!)!.runs, picks[0][0])!, 'both'), 0);
    assert.equal(navalValue(runAt(model.series.get('mod country A')!.runs, january)!, 'both'), 6);
  } finally { await vite.close(); }
});

test('an open naval drill keeps its chosen country and full history after Top N or group exit', async () => {
  const countryIdentities = ['mod country A', 'mod country B'].map((tag, slot) => ({ tag,
    identities: [{ firstHour: 100, lastHour: 105, slot, countryId: slot + 10 }] }));
  const series = (tag: string, firstHour: number, lastHour: number, ships: number) => ({ tag, membership: null,
    identities: countryIdentities.find(history => history.tag === tag)!.identities,
    segments: [{ firstHour: 100, lastHour: 105 }], runs: [{ firstHour, lastHour, startsSegment: true, ships, convoys: 2 }] });
  const reply: NavalLossesReply = { branchId: '1', available: true, firstRead: 100, shipFirstRead: 100, convoyFirstRead: 100,
    shipLastRead: 105, convoyLastRead: 105, shipReads: [100, 105], convoyReads: [100, 105], warnings: [],
    series: [series('mod country A', 100, 101, 4), series('mod country B', 100, 105, 8)] };
  const full = { ...reply, series: [series('mod country A', 100, 105, 4)] };
  const groups = { warnings: [], countryIdentities, humans: [{ tag: 'mod country A', firstHour: 100, lastHour: 101 },
    { tag: 'mod country B', firstHour: 100, lastHour: 105 }], unobserved: { humans: [], factions: [], wars: [] } };
  const replacements: Record<string, string> = {
    '/dashboard/productWindows.ts': navalWindowFixture,
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.params.top === null
      && request.params.tags.length === 1 && request.params.tags[0] === 'mod country A' ? ${JSON.stringify(full)} : ${JSON.stringify(reply)} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/components/CountryPicker.tsx': `import { createElement } from 'react'; export default props => createElement('button', { 'data-selected': props.selectedTag }, props.selectedTag);`,
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: ${JSON.stringify(groups)} });`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456", "#654321"] });',
    '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react';
      export default function View(p) { return p.view === 'mix' ? createElement('section', { 'data-country': p.mix?.tag,
        'data-bands': JSON.stringify(p.mix?.bands) }, p.beforeChart) : null; }`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'naval-selected-history', enforce: 'pre', transform: (source, id) => {
      // Start with a country already picked, keeping the real reducer and widget hooks.
      if (id.replaceAll('\\', '/').endsWith('/charts/drill/drillState.ts')) return source.replace(
        'export const CLOSED: DrillState = { breakdown: null, follow: true, hour: null };',
        'export const CLOSED: DrillState = { breakdown: "mod country A", follow: true, hour: null };');
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx');
    const track = { id: 'track', source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
      branch: { id: '1', firstHour: 100, lastHour: 105 } };
    const runtime = { tracks: [track], client: {}, clock: { getHour: () => 105 } };
    for (const countries of [definition.defaultCountries, { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }]) {
      const widget = { id: 'naval', kind: 'naval-losses', trackIds: ['track'], countries, time: { mode: 'campaign' }, options: { show: 'parts' } };
      const html = renderToStaticMarkup(createElement(Widget, { widget, runtime, active: true, onDiagnostics() {} }));
      assert.match(html, /data-country="mod country A"/);
      assert.match(html, /data-selected="mod country A"/);
      assert.match(html, /Ships<\/span>.*?<b>4<\/b>/);
      assert.match(html, /Convoys<\/span>.*?<b>2<\/b>/);
      assert.match(html, /&quot;lastHour&quot;:105/);
      assert.doesNotMatch(html, /Unknown/);
    }
  } finally { await vite.close(); }
});

test('Pin as widget keeps the picked February hour rather than the March display or January read', async () => {
  const firstHour = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  const readHour = engineHour({ year: 1936, month: 1, day: 31, hour: 24 })!;
  const pickedHour = readHour + 1, displayedHour = engineHour({ year: 1936, month: 3, day: 1, hour: 1 })!;
  const reply: NavalLossesReply = { branchId: '1', available: true, firstRead: readHour, shipFirstRead: readHour,
    convoyFirstRead: readHour, shipLastRead: readHour, convoyLastRead: readHour, shipReads: [readHour], convoyReads: [readHour],
    warnings: [], series: [{ tag: 'mod country A', membership: null,
      identities: [{ firstHour: readHour, lastHour: displayedHour, slot: 0, countryId: 10 }],
      segments: [{ firstHour: readHour, lastHour: displayedHour }], runs: [
        { firstHour: readHour, lastHour: readHour, startsSegment: true, ships: 4, convoys: 2 },
        { firstHour: pickedHour, lastHour: displayedHour, startsSegment: false, ships: 0, convoys: 0 },
      ] }] };
  assert.equal(navalReadHour(reply, pickedHour, 'both'), readHour);
  const replacements: Record<string, string> = {
    '/dashboard/productWindows.ts': navalWindowFixture,
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(reply)} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456", "#654321"] });',
    '/charts/race/CountryRaceView.tsx': 'export default function View() { return null; }',
    '/charts/drill/DrillHost.tsx': 'export default function Host(p) { p.onPin?.(); return null; }',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'naval-pin-picked-day', enforce: 'pre', transform: (source, id) => {
      if (id.replaceAll('\\', '/').endsWith('/charts/drill/drillState.ts')) return source.replace(
        'export const CLOSED: DrillState = { breakdown: null, follow: true, hour: null };',
        `export const CLOSED: DrillState = { breakdown: "mod country A", follow: false, hour: ${pickedHour} };`);
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
    } }] });
  try {
    // Pin uses Vite's widget registry, separate from Node's imported metadata.
    await vite.ssrLoadModule('/ui/src/widgets/definitions.ts');
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx');
    const track = { id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
      branch: { id: '1', firstHour, lastHour: displayedHour } };
    const created: WidgetSpec[] = [];
    const runtime = { tracks: [track], client: {}, clock: { getHour: () => displayedHour },
      addWidgetBelow: (_id: string, pinned: WidgetSpec) => created.push(pinned) };
    for (const time of [{ mode: 'campaign' }, { mode: 'range', firstOffset: 0, lastOffset: displayedHour - firstHour }]) {
      const widget = { id: 'naval', kind: 'naval-losses', title: 'Naval losses', trackIds: ['track'],
        countries: definition.defaultCountries, time, options: { period: 'month' } };
      renderToStaticMarkup(createElement(Widget, { widget, runtime, active: true, onDiagnostics() {} }));
      const pinned = created.at(-1)!;
      assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['mod country A'] });
      assert.equal(pinned.options?.show, 'parts');
      assert.deepEqual(pinned.time, { mode: 'pin', windows: { track: { firstOffset: pickedHour - firstHour, lastOffset: pickedHour - firstHour } } });
      assert.equal(resolveWindow(pinned, track as never, [track] as never, displayedHour)?.lastHour, pickedHour);
    }
    assert.equal(created.length, 2);
  } finally { await vite.close(); }
});

test('a queued drill history pick survives playback and artwork updates but rejects changed quantities', async () => {
  const reply: NavalLossesReply = { branchId: '1', available: true, firstRead: 100, shipFirstRead: 100, convoyFirstRead: 100,
    shipLastRead: 110, convoyLastRead: 110, shipReads: [100, 110], convoyReads: [100, 110], warnings: [],
    series: [{ tag: 'mod country A', membership: null, segments: [{ firstHour: 100, lastHour: 110 }],
      runs: [{ firstHour: 100, lastHour: 110, startsSegment: true, ships: 4, convoys: 2 }] }] };
  const state = { reply, hour: 102, name: 'Before', flag: undefined as string | undefined,
    charts: [] as TimeChartProps[], drill: { breakdown: 'mod country A', follow: true, hour: null, choices: null } as PartsRaceDrillState,
    memos: new Map<string, { deps: readonly unknown[]; value: unknown }[]>(), cursors: new Map<string, number>() };
  const host = globalThis as typeof globalThis & { __navalPick?: typeof state;
    __navalMemo?: (id: string, factory: () => unknown, deps: readonly unknown[]) => unknown;
    __navalReducer?: (reducer: typeof reducePartsRaceDrill) => [PartsRaceDrillState, (action: Parameters<typeof reducePartsRaceDrill>[1]) => void] };
  host.__navalPick = state;
  // Retain mounted memo/reducer slots while keeping the actual widget, shared
  // owner resolver, numeric bands and drawn callback; SSR runs no chart effects.
  host.__navalMemo = (id, factory, deps) => {
    const index = state.cursors.get(id) ?? 0; state.cursors.set(id, index + 1);
    let slots = state.memos.get(id);
    if (!slots) { slots = []; state.memos.set(id, slots); }
    const before = slots[index];
    if (before && before.deps.length === deps.length && before.deps.every((value, i) => Object.is(value, deps[i]))) return before.value;
    const value = factory(); slots[index] = { deps: [...deps], value }; return value;
  };
  host.__navalReducer = reducer => [state.drill, action => { state.drill = reducer(state.drill, action); }];
  const replacements: Record<string, string> = {
    '/dashboard/productWindows.ts': navalWindowFixture,
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__navalPick.reply }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackMotion = () => 1;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => globalThis.__navalPick.name + tag,
      color: () => [50, 100, 150], flag: () => globalThis.__navalPick.flag, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({
      name: globalThis.__navalPick.name + tag, flag: globalThis.__navalPick.flag }), revision: 0 });`,
    '/charts/useChartTokens.ts': `const tokens = { categories: ['#123456', '#654321'], muted: '#888888',
      surface: '#000000', countryColours: 'raw', fontSize: { body: 12 } };
      export const useChartTokens = () => tokens; export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__navalPick.charts.push(props); return null; };',
    '/components/CountryPicker.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'naval-queued-history-pick', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        const module = ['/widgets/naval-losses/NavalLossesWidget.tsx', '/charts/race/CountryRaceView.tsx'].find(suffix => path.endsWith(suffix));
        if (module) {
          source = source.replace(/\buseMemo\(/g, `globalThis.__navalMemo(${JSON.stringify(module)}, `);
          if (module.endsWith('/NavalLossesWidget.tsx')) source = source.replace(/\buseReducer\(/g, 'globalThis.__navalReducer(');
          return source;
        }
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx');
    const track = { id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
      branch: { id: '1', firstHour: 100, lastHour: 110 } };
    const props = { widget: { id: 'naval', kind: 'naval-losses', trackIds: ['track'], countries: definition.defaultCountries,
      time: { mode: 'follow' }, options: { show: 'parts' } },
      runtime: { tracks: [track], client: {}, clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: true }) } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const picks = new ChartPicks();
    const render = () => {
      state.charts = []; state.cursors.clear();
      renderToStaticMarkup(createElement(Widget, props));
      const chart = state.charts.at(-1)!;
      assert.ok(chart?.onPick);
      picks.update({ owner: chart.pickOwner, extent: chart.extent, branchStart: chart.branchStart, active: chart.active, onPick: chart.onPick });
      return chart;
    };
    const first = render();
    const context = { frame: plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [100, 110], [0, 10], false), until: 102 };
    const queued = picks.drawn(context, first.onPick);
    state.hour = 103; state.name = 'After'; state.flag = 'new artwork';
    const after = render();
    assert.equal(after.pickOwner, first.pickOwner, 'presentation and playback preserve the mounted numeric owner');
    assert.equal(picks.pick(queued, 101), true, 'the pending click retains its drawn hour and callback');
    assert.deepEqual(state.drill, { breakdown: 'mod country A', follow: false, hour: 101, choices: null });
    const obsolete = picks.drawn(context, after.onPick);
    state.reply = structuredClone(reply); state.reply.series[0].runs[0].ships = 5;
    const changed = render();
    assert.notEqual(changed.pickOwner, after.pickOwner);
    assert.equal(picks.pick(obsolete, 101), false, 'a real numeric model change still cancels the queued pick');
  } finally {
    await vite.close(); delete host.__navalPick; delete host.__navalMemo; delete host.__navalReducer;
  }
});

test('the actual Naval adapter holds overlapping drawn choices, follows the aligned clock and rejects obsolete owners', async () => {
  const reply: NavalLossesReply = { branchId: '1', available: true, firstRead: 100, shipFirstRead: 100, convoyFirstRead: 100,
    shipLastRead: 110, convoyLastRead: 110, shipReads: [100, 110], convoyReads: [100, 110], warnings: [],
    series: ['mod A', 'mod B', 'mod C'].map((tag, index) => ({ tag, membership: null, segments: [{ firstHour: 100, lastHour: 110 }],
      identities: [{ firstHour: 100, lastHour: 110, slot: index, countryId: index + 10 }],
      runs: [{ firstHour: 100, lastHour: 110, startsSegment: true, ships: index < 2 ? 6 : 12, convoys: -2 }] })) };
  const state = { hooks: [] as unknown[], cursor: 0, reply, detailReply: undefined as NavalLossesReply | undefined, hour: 3, playing: false,
    wanted: [] as { tag: string; hour: number }[],
    main: undefined as CountryRaceViewProps | undefined, detail: undefined as CountryRaceViewProps | undefined,
    host: undefined as ComponentProps<typeof DrillHost> | undefined, picker: undefined as ComponentProps<typeof CountryPicker> | undefined,
    added: [] as WidgetSpec[] };
  const globals = globalThis as typeof globalThis & { __navalAdapter?: typeof state };
  globals.__navalAdapter = state;
  // Retain only the actual adapter's hook cells between SSR draws, as a mount does.
  const hooks = `const cell = init => { const state = globalThis.__navalAdapter, index = state.cursor++; return state.hooks[index] ??= init(); };
    export const useEffect = () => {};
    export const useRef = initial => cell(() => ({ current: initial }));
    export const useMemo = (make, deps) => { const saved = cell(() => ({ deps, value: make() }));
      if (saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) { saved.deps = deps; saved.value = make(); }
      return saved.value; };
    export const useCallback = (callback, deps) => useMemo(() => callback, deps);
    export const useState = initial => { const saved = cell(() => ({ value: typeof initial === 'function' ? initial() : initial }));
      return [saved.value, next => { saved.value = typeof next === 'function' ? next(saved.value) : next; }]; };
    export const useReducer = (reduce, initial) => { const saved = cell(() => ({ value: initial }));
      return [saved.value, action => { saved.value = reduce(saved.value, action); }]; };`;
  const replacements: Record<string, string> = {
    '/dashboard/productWindows.ts': navalWindowFixture,
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request ?
      request.params.top === null && request.params.tags.length === 1 && globalThis.__navalAdapter.detailReply
        ? globalThis.__navalAdapter.detailReply : globalThis.__navalAdapter.reply : undefined });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted, active) => {
      if (active) globalThis.__navalAdapter.wanted.push(...wanted); return { labelAt: hour => tag => ({ name: tag + '@' + hour }), revision: 0 }; };`,
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456", "#654321"] });',
    '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__navalAdapter[props.view === 'mix' ? 'detail' : 'main'] = props; return createElement('div', null, props.beforeChart); };`,
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__navalAdapter.picker = props; return null; };',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__navalAdapter.host = props; return createElement('div', null, props.main, props.open ? props.children : null); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'naval-drawn-actor-fixture', enforce: 'pre',
      resolveId(id) { if (id === 'naval-adapter-hooks') return '\0naval-adapter-hooks'; },
      load(id) { if (id === '\0naval-adapter-hooks') return hooks; },
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/naval-losses/NavalLossesWidget.tsx')) return source.replace("from 'react'", "from 'naval-adapter-hooks'");
      },
    }] });
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const metadata = await vite.ssrLoadModule('/ui/src/widgets/naval-losses/definition.ts');
    registry.registerWidgets([metadata.definition]);
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx')).default as ComponentType<WidgetProps>;
    const track = { id: 'track', axisOffset: 100, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
      branch: { id: '1', firstHour: 100, lastHour: 110 } };
    const props = { widget: { id: 'naval', kind: 'naval-losses', title: 'Naval losses', trackIds: ['track'],
      countries: { kind: 'tags', tags: ['mod A', 'mod B', 'mod C'] }, time: { mode: 'campaign' }, options: { view: 'lines' } },
      runtime: { tracks: [track], client: {}, clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: state.playing }) },
        addWidgetBelow: (_id: string, widget: WidgetSpec) => state.added.push(widget) }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => {
      state.cursor = 0; state.main = undefined; state.detail = undefined; state.host = undefined; state.picker = undefined; state.wanted = [];
      return renderToStaticMarkup(createElement(Widget, props));
    };
    render();
    const frame = plotFrame(400, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [100, 110], [0, 12], false);
    const drawn = state.main!.lines.map(line => ({ key: line.tag, runs: line.runs, held: false }));
    const choices = pickLines(drawn, 104, { frame, until: null, point: { x: frame.toX(104), y: frame.toY(4) } });
    assert.deepEqual(choices, ['mod A', 'mod B']);
    state.main!.onPick!(104, undefined, choices); render();
    assert.deepEqual(state.picker!.allowedTags, choices, 'the actual adapter passes every overlapping drawn actor to the chooser');
    assert.equal(state.host!.onPin, undefined); assert.equal(state.host!.canFollow, false);
    const firstChooser = state.picker!.onChange;
    state.hour = 8; render(); assert.equal(state.picker!.flagAt!.hour, 100, 'chooser artwork keeps its supplying read while its chosen day remains104');
    assert.deepEqual([...state.picker!.choiceReads!], choices.map(tag => [tag, { tag, branchId: '1', hour: 100 }]));
    state.picker!.onChange('mod B'); const html = render();
    assert.equal(state.detail!.mix!.tag, 'mod B'); assert.equal(state.detail!.moment, 104);
    assert.equal(state.main!.active, false);
    assert.match(html, /Convoys<\/span>.*?<b class="is-negative">-2<\/b>/, 'correction rows keep their sign');
    assert.match(html, /Convoys<\/span>.*?<small>-50%<\/small>/, 'signed corrections retain their negative share of the signed total');
    assert.equal(state.detail!.mix!.bands![1][0].value, -2, 'a signed correction is drawn below zero, as recorded');
    state.host!.onPin!(); assert.deepEqual(state.added[0].countries, { kind: 'tags', tags: ['mod B'] });
    assert.deepEqual(state.added[0].time, { mode: 'pin', windows: { track: { firstOffset: 4, lastOffset: 4 } } });
    state.host!.onFollow(); render(); assert.equal(state.detail!.moment, 108, 'Campaign Follow uses the actual aligned clock');
    state.hour = 9; render(); assert.equal(state.detail!.moment, 109);
    state.detail!.onPick!(103); render(); assert.equal(state.detail!.moment, 103);
    firstChooser('mod A'); render(); assert.equal(state.detail!.mix!.tag, 'mod B', 'a settled chooser cannot change a later drill');
    state.host!.onClose(); render();
    const nearest = pickLines(drawn, 104, { frame, until: null, point: { x: frame.toX(104), y: frame.toY(10) } });
    assert.deepEqual(nearest, ['mod C']);
    state.main!.onPick!(104, nearest[0], nearest); render(); assert.equal(state.detail!.mix!.tag, 'mod C');
    state.host!.onClose(); render(); state.main!.onPick!(104, undefined, []); render(); assert.equal(state.host!.open, false, 'empty drawn choices never select a race leader');
    const obsolete = state.main!.onPick!, originalClient = props.runtime.client;
    props.runtime.client = {} as never; render(); props.runtime.client = originalClient; render();
    obsolete(104, 'mod A'); render(); assert.equal(state.host!.open, false, 'A to B to A clients cannot restore an old callback');
    const hidden = state.main!.onPick!; props.active = false; render(); hidden(104, 'mod A'); render(); assert.equal(state.host!.open, false);
    props.active = true; render(); hidden(104, 'mod A'); render(); assert.equal(state.host!.open, false, 'visibility reentry also invalidates old draws');
    const unavailable = state.main!.onPick!;
    state.reply = { ...reply, available: false }; render(); unavailable(104, 'mod A'); render(); assert.equal(state.host!.open, false);
    state.reply = reply; render(); unavailable(104, 'mod A'); render(); assert.equal(state.host!.open, false);
    const changed = state.main!.onPick!;
    state.reply = structuredClone(reply); state.reply.series[0].runs[0].ships = 7; render(); changed(104, 'mod A'); render(); assert.equal(state.host!.open, false);
    state.main!.onPick!(104, undefined, ['mod A', 'mod B']); render();
    const changedChooser = state.picker!.onChange;
    props.widget.options = { ...props.widget.options, period: 'month' }; render();
    changedChooser('mod A'); render(); assert.equal(state.host!.open, false, 'an old selected measure cannot settle a chooser');
    state.reply = reply; state.hour = 8;
    props.widget.options = { ...props.widget.options, period: 'running' };
    props.widget.time = { mode: 'range', firstOffset: 0, lastOffset: 4 }; render();
    state.main!.onPick!(104, undefined, ['mod A', 'mod B']); render(); state.picker!.onChange('mod B'); render();
    assert.deepEqual(state.main!.extent, [100, 104], 'the picked range retains its original basis');
    assert.deepEqual(state.detail!.extent, [100, 110], 'the detail history has stable branch bounds');
    assert.equal(state.detail!.moment, 104); assert.equal(state.host!.canFollow, true, 'the available branch clock is beyond the range end');
    state.host!.onFollow(); render();
    assert.equal(state.detail!.moment, 108); assert.equal(state.detail!.state, undefined, 'Range Follow can show the available branch clock');
    assert.deepEqual(state.main!.extent, [100, 104], 'the source range stays selected');
    assert.deepEqual(state.detail!.extent, [100, 110]);
    assert.equal(state.detail!.mix!.bands![0].at(-1)!.lastHour, 110, 'the followed read has history beyond the original range end');
    const followedOwner = state.detail!.pickOwner;
    state.hour = 9; render(); assert.equal(state.detail!.moment, 109);
    assert.deepEqual(state.detail!.extent, [100, 110], 'Follow ticks keep the extent that preserves chart zoom');
    assert.equal(state.detail!.pickOwner, followedOwner);
    state.detail!.onPick!(103); render(); assert.equal(state.detail!.moment, 103);
    assert.deepEqual(state.detail!.extent, [100, 110], 'a historical pick does not shrink the zoom scope');
    state.detail!.onPick!(107); render(); assert.equal(state.detail!.moment, 107); assert.equal(state.detail!.state, undefined);
    state.host!.onPin!(); assert.deepEqual(state.added.at(-1)!.time, { mode: 'pin', windows: { track: { firstOffset: 7, lastOffset: 7 } } });
    state.host!.onClose();
    const native = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ firstHour, lastHour, slot, countryId });
    state.reply = { ...reply, series: [
      { tag: 'mod A', membership: null, segments: [{ firstHour: 100, lastHour: 104 }, { firstHour: 105, lastHour: 110 }],
        identities: [native(100, 104, 1, 11), native(105, 110, 2, 22)], runs: [
          { firstHour: 100, lastHour: 104, startsSegment: true, ships: 4, convoys: -2 },
          { firstHour: 105, lastHour: 110, startsSegment: true, ships: 100, convoys: 0 },
        ] },
      { tag: 'alias', membership: null, segments: [{ firstHour: 105, lastHour: 108 }, { firstHour: 109, lastHour: 110 }],
        identities: [native(105, 108, 1, 11), native(109, 110, 3, 33)], runs: [
          { firstHour: 105, lastHour: 108, startsSegment: true, ships: 5, convoys: -2 },
          { firstHour: 109, lastHour: 110, startsSegment: true, ships: 200, convoys: 0 },
        ] },
    ] };
    props.widget.countries = { kind: 'tags', tags: ['mod A'] }; props.widget.time = { mode: 'campaign' };
    props.widget.options = { ...props.widget.options, period: 'running' }; state.hour = 8; render();
    assert.deepEqual(state.main!.ranked.map(row => row.tag), ['mod A'], 'the unrelated holder of alias is outside the literal root selection');
    state.main!.onPick!(104, 'mod A', ['mod A']); render(); state.host!.onFollow(); render();
    assert.equal(state.detail!.mix!.tag, 'alias', 'Follow keeps the chosen native holder through its recorded rename');
    assert.ok(state.detail!.mix!.bands![0].every(run => run.value < 100), 'the old literal tag and reused alias cannot contribute a different holder');
    assert.deepEqual(state.detail!.tooltipTags!(103, ['alias']), ['mod A']);
    const oldTips = [...(state.detail!.tooltip('alias', 103, 'alias', new Map(), () => tag => ({ name: tag })) ?? [])];
    assert.equal(oldTips[0].title, 'mod A', 'historical tooltips keep the selected holder’s earlier alias');
    assert.equal(oldTips[0].rows!.find(row => row.label === 'Ships')!.value, '4');
    state.host!.onPin!(); assert.deepEqual(state.added.at(-1)!.countries, { kind: 'tags', tags: ['alias'] });
    state.detail!.onPick!(109); render();
    assert.equal(state.detail!.moment, 109);
    assert.ok(state.detail!.state, 'a picked moment without the native holder stays unavailable');
    const refusedFollow = state.host!.onFollow;
    assert.equal(state.host!.canFollow, true, 'the available live clock can recover an unknown selected moment');
    state.host!.onFollow(); render();
    assert.equal(state.detail!.moment, 108);
    assert.equal(state.detail!.mix!.tag, 'alias', 'Follow recovery retains the original chosen native holder');
    assert.equal(state.detail!.state, undefined);
    state.hour = 9; render(); assert.ok(state.detail!.state, 'a native identity gap is unavailable rather than selecting the alias replacement');
    assert.equal(state.host!.onPin, undefined);
    state.host!.onClose();
    const recoveryClient = props.runtime.client;
    props.runtime.client = {} as never; render(); refusedFollow(); render();
    assert.equal(state.host!.open, false, 'an old recovery callback cannot reopen a drill after client replacement');
    props.runtime.client = recoveryClient; render(); refusedFollow(); render();
    assert.equal(state.host!.open, false, 'returning to the old client cannot restore its obsolete recovery callback');
    const january = engineHour({ year: 1936, month: 1, day: 31, hour: 24 })!, february = january + 1;
    track.branch.firstHour = january; track.branch.lastHour = february + 2; state.hour = february - track.axisOffset;
    state.reply = { ...reply, firstRead: january, shipFirstRead: january, convoyFirstRead: january,
      shipLastRead: january, convoyLastRead: january, shipReads: [january], convoyReads: [january],
      series: ['mod A', 'mod B'].map((tag, index) => ({ tag, membership: null, segments: [{ firstHour: january, lastHour: february + 2 }],
        identities: [native(january, february + 2, index, index + 10)], runs: [
        { firstHour: january, lastHour: january, startsSegment: true, ships: 4, convoys: 2 },
        { firstHour: february, lastHour: february + 2, startsSegment: false, ships: 0, convoys: 0 },
      ] })) };
    props.widget.countries = { kind: 'tags', tags: ['mod A', 'mod B'] };
    props.widget.time = { mode: 'campaign' }; props.widget.options = { ...props.widget.options, period: 'month' }; render();
    state.main!.onPick!(february, undefined, ['mod A', 'mod B']); render();
    assert.equal(state.picker!.flagAt!.hour, january, 'a February zero-month chooser names the countries at the actual January supplying read');
    state.picker!.onChange('mod B'); render(); assert.equal(state.detail!.moment, february, 'the picked day remains February');
    state.host!.onPin!(); assert.deepEqual(state.added.at(-1)!.time, { mode: 'pin', windows: { track: { firstOffset: 1, lastOffset: 1 } } });
    state.host!.onClose();
    track.branch.firstHour = 100; track.branch.lastHour = 110; state.hour = 8;
    const country = (tag: string, identities: NonNullable<NavalLossesReply['series'][number]['identities']>) => ({ tag, identities,
      membership: null, segments: identities.map(({ firstHour, lastHour }) => ({ firstHour, lastHour })),
      runs: identities.map(({ firstHour, lastHour }) => ({ firstHour, lastHour, startsSegment: true, ships: 4, convoys: -2 })) });
    state.reply = { ...reply, shipReads: [100], convoyReads: [100], shipLastRead: 100, convoyLastRead: 100, series: [
      country('old A', [native(100, 103, 1, 11)]), country('mod A', [native(104, 110, 1, 11)]),
      country('old B', [native(100, 103, 2, 22)]), country('mod B', [native(104, 110, 2, 22)]),
      country('unproved', [native(104, 110, 3, 33)]),
    ] };
    state.detailReply = { ...state.reply, shipReads: [100, 102], convoyReads: [100, 102], shipLastRead: 102, convoyLastRead: 102,
      series: state.reply.series.filter(series => series.tag === 'old A' || series.tag === 'mod A') };
    props.widget.countries = { kind: 'tags', tags: ['mod A', 'mod B', 'unproved'] };
    props.widget.time = { mode: 'campaign' }; props.widget.options = { ...props.widget.options, period: 'running' }; render();
    state.main!.onPick!(108, undefined, ['mod A', 'mod B', 'unproved']); render();
    assert.deepEqual([...state.picker!.choiceReads!], [
      ['mod A', { tag: 'old A', branchId: '1', hour: 100 }], ['mod B', { tag: 'old B', branchId: '1', hour: 100 }], ['unproved', null],
    ], 'overlap artwork uses each supplying alias while callbacks retain the drawn actor');
    state.picker!.onChange('mod A'); render();
    assert.deepEqual([...state.picker!.choiceReads!], [
      ['mod A', { tag: 'old A', branchId: '1', hour: 102 }], ['mod B', { tag: 'old B', branchId: '1', hour: 100 }], ['unproved', null],
    ], 'detail choices use their own model and supplying read, rather than the selected actor\'s date');
    assert.deepEqual(state.wanted, [{ tag: 'old A', hour: 102 }], 'the adapter requests only its selected title; Picker owns visible choices');
    assert.match(state.host!.title, /old A@102/);
    const oldSelector = state.picker!.onChange;
    state.host!.onPin!(); assert.deepEqual(state.added.at(-1)!.countries, { kind: 'tags', tags: ['mod A'] });
    assert.deepEqual(state.added.at(-1)!.time, { mode: 'pin', windows: { track: { firstOffset: 8, lastOffset: 8 } } });
    state.detailReply = undefined;
    state.picker!.onChange('mod B'); render(); assert.equal(state.detail!.mix!.tag, 'mod B', 'selection returns the drawn tag, not old B');
    oldSelector('mod A'); render(); assert.equal(state.detail!.mix!.tag, 'mod B', 'obsolete detail selectors cannot replace the chosen actor');
    state.host!.onClose();
    state.reply = { ...state.reply, series: [
      country('mod A', [native(104, 110, 1, 11)]),
      country('reused alias', [native(100, 103, 1, 11), native(104, 110, 2, 22)]),
      country('old B', [native(100, 103, 2, 22)]),
    ] };
    state.detailReply = { ...state.reply, shipReads: [100, 102], convoyReads: [100, 102], shipLastRead: 102, convoyLastRead: 102, series: [
      country('mod A', [native(104, 110, 1, 11)]), country('reused alias', [native(100, 103, 1, 11)]),
    ] };
    props.widget.countries = { kind: 'tags', tags: ['mod A', 'old B'] }; render();
    state.main!.onPick!(108, 'mod A', ['mod A']); render();
    assert.deepEqual(state.picker!.choiceReads!.get('reused alias'), { tag: 'old B', branchId: '1', hour: 100 },
      'a chosen holder\'s earlier alias cannot suppress another selected holder now using it');
    state.detailReply = undefined;
    state.picker!.onChange('reused alias'); render();
    assert.equal(state.detail!.mix!.tag, 'reused alias', 'that current holder remains selectable through the main numeric model');
    state.host!.onPin!(); assert.deepEqual(state.added.at(-1)!.countries, { kind: 'tags', tags: ['reused alias'] });
  } finally { await vite.close(); delete globals.__navalAdapter; }
});

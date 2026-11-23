import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, isValidElement } from 'react';
import type { ComponentType, ComponentProps } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type CountryRaceView from '../charts/race/CountryRaceView';
import type CountryPicker from '../components/CountryPicker';
import type DrillHost from '../charts/drill/DrillHost';
import type DrillParts from '../charts/drill/DrillParts';
import type TimeChart from '../charts/TimeChart';
import type DateTimePicker from '../components/DateTimePicker';
import { plotFrame } from '../charts/frame.ts';
import { pickLines } from '../charts/linePick.ts';
import { tokensFromValues } from '../charts/tokens.ts';
import type { WidgetProps } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { persistedSpec, type PinnedSpec } from './regionTime.ts';
import { gatedHour } from '../test-support/gated.ts';
import type BombingDrill from './BombingDrill.tsx';
import type AirLossDrill from './AirLossDrill.tsx';
import type { SunburstProps } from '../charts/sunburst/Sunburst';
import { militaryAirIndex, validateMilitaryAirReply } from './militaryAirLedger.ts';
import { OPTIONS } from '../widgets/air-kills-losses/definition.ts';
import { partOf } from './militaryAirParts.fixture.ts';
import type { Leaf } from './militaryAirParts.fixture';
import type { MilitaryAirLedger } from './militaryAirLedger';

const span = (firstHour: number, lastHour = firstHour) => ({ firstHour, lastHour });
const data = (): MilitaryAirLedger => ({ family: 'airLosses', firstHour: 10, lastHour: 20, available: true,
  observed: [span(10, 16), span(18, 20)], incomplete: [], countries: ['A', 'B', 'C'].map((tag, id) => ({ tag, identity: `${id}:9`,
    present: [span(10, 16), span(18, 20)], membership: tag === 'A' ? [span(10, 16), span(18, 20)] : tag === 'B' ? [span(10, 12)] : [span(13, 16), span(18, 20)] })), parts: ([
    { tag: 'B', identity: '1:9', dealt: false, opponent: null, region: 40001, cause: 1, buildingToken: null, buildingId: null, state: null,
      points: [[11, 16, '900719925474099300000'], [12, 18, '900719925474099200000']] },
    { tag: 'B', identity: '1:9', dealt: false, opponent: null, region: 40001, cause: 3, buildingToken: null, buildingId: null, state: null,
      points: [[11, 16, '-100000']] },
    ...(['A', 'C'] as const).map(tag => ({ tag, identity: tag === 'A' ? '0:9' : '2:9', dealt: true, opponent: null, region: 40001, cause: 1,
      buildingToken: null, buildingId: null, state: null, points: [[11, tag === 'A' ? 11 : 13, tag === 'A' ? '200000' : '100000'] as [number, number, string]] })),
  ] as Leaf[]).map(partOf), regions: [], buildings: [], warnings: [] });

const hooks = `const state = () => globalThis.__airDrill;
  const cell = init => { const s = state(), i = s.cursor++; return s.hooks[i] ??= init(); };
  export const useEffect = () => {};
  export const useRef = initial => cell(() => ({ current: initial }));
  export const useMemo = (make, deps) => { const value = cell(() => ({ deps, value: make() }));
    if (value.deps.length !== deps.length || value.deps.some((dep, i) => !Object.is(dep, deps[i]))) { value.deps = deps; value.value = make(); }
    return value.value; };
  export const useState = initial => { const value = cell(() => ({ value: typeof initial === 'function' ? initial() : initial }));
    return [value.value, next => { value.value = typeof next === 'function' ? next(value.value) : next; }]; };
  export const useReducer = (reduce, initial) => { const value = cell(() => ({ value: initial }));
    return [value.value, action => { value.value = reduce(value.value, action); }]; };
  export const useCallback = (callback, deps) => useMemo(() => callback, deps);
  export const useLayoutEffect = () => {};
  export const useSyncExternalStore = (_subscribe, read) => read();
  export { createContext, createElement, useContext } from 'react';`;

function fixture() {
  return { hooks: [] as unknown[], cursor: 0, data: validateMilitaryAirReply(data()),
    track: { id: 'track', axisOffset: 1000, source: { id: 'source', revision: 1, metadata: { countries: [], campaignKey: 'campaign' } },
      branch: { id: 'branch', firstHour: 10, lastHour: 20, gaps: [span(17)] } }, hour: -988, playing: false, added: [] as WidgetSpec[],
    groups: { retry() {} } as { retry(): void; data?: unknown },
    bombing: undefined as ComponentProps<typeof BombingDrill> | undefined,
    butterfly: undefined as { items: unknown[]; active: boolean; onSelect: (item: unknown, side: 'left' | 'right') => void } | undefined,
    chart: undefined as ComponentProps<typeof CountryRaceView> | undefined, host: undefined as (ComponentProps<typeof DrillHost> & { open: boolean; onFollow: () => void; onPause: () => void; following: boolean }) | undefined,
    picker: undefined as ComponentProps<typeof CountryPicker> | undefined, detail: undefined as ComponentProps<typeof AirLossDrill> | undefined,
    parts: undefined as ComponentProps<typeof DrillParts> | undefined, history: undefined as ComponentProps<typeof TimeChart> | undefined,
    datePicker: undefined as ComponentProps<typeof DateTimePicker> | undefined, reset: undefined as (() => void) | undefined };
}
async function server(replacements: Record<string, string>) {
  return createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'air-drill-source', enforce: 'pre',
      resolveId(id) { if (id === 'air-drill-hooks') return '\0air-drill-hooks'; },
      load(id) { if (id === '\0air-drill-hooks') return hooks; },
      transform(code, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/dashboard/MilitaryAirView.tsx') || path.endsWith('/dashboard/AirLossDrill.tsx') || path.endsWith('/dashboard/BombingDrill.tsx')
          || path.endsWith('/charts/drill/useDrill.ts') || path.endsWith('/dashboard/widgetTime.ts'))
          return code.replace("from 'react'", "from 'air-drill-hooks'");
      },
    }] });
}
const common = {
  '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 24;',
  '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: (tag, hour) => tag + " at " + hour, color: () => null, flag: () => undefined, flagReason: () => undefined, flagPending: () => false, revision: 0 });',
  '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = (_client, _track, wanted, active) => { if (active || wanted.length) throw new Error("This fixture has no country header target"); return { labelAt: () => () => { throw new Error("No country header label was requested"); }, revision: 0 }; };',
  '/charts/useChartTokens.ts': `const tokens = ${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))}; export const useChartTokens = () => tokens;`,
};

test('bombing breakdown keeps damage amounts and uses readable missing-content labels', async () => {
  const state = fixture();
  const host = globalThis as typeof globalThis & { __airDrill?: ReturnType<typeof fixture>; __bombingBreakdown?: SunburstProps };
  host.__airDrill = state;
  const reply = data(); reply.family = 'bombing';
  reply.buildings = [{id: 'synthetic-private-token', name: null, missing: 'Synthetic raw building lookup refusal'}];
  reply.parts = reply.parts.map(part => ({...part, opponentKnown: null, buildingToken: 'synthetic-private-token', buildingId: 1, state: 0}));
  const vite = await server({...common,
    '/dashboard/products.ts': 'export const useProduct = (client, request) => request ? {error: new Error("Synthetic raw place-name failure")} : {}; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/charts/sunburst/Sunburst.tsx': 'export default props => {globalThis.__bombingBreakdown = props; return null;};',
  });
  try {
    const {default: Breakdown} = await vite.ssrLoadModule('/ui/src/widgets/bombing-damage-breakdown/BombingBreakdown.tsx') as typeof import('../widgets/bombing-damage-breakdown/BombingBreakdown.tsx');
    renderToStaticMarkup(createElement(Breakdown, {data: reply, runtime: {client: {}, clock: {}} as never, track: state.track as unknown as BoundTrack,
      tag: 'B', identity: '1:9', namedAt: gatedHour(20), from: 10, hour: 19, end: 20, dealt: false, options: {}, active: true}));
    const chart = host.__bombingBreakdown!;
    assert.equal(chart.text.valueLabel, 'Damage'); assert.equal(chart.text.countLabel, 'Locations');
    assert.equal(chart.text.unit, 'building levels'); assert.match(chart.footnote as string, /Some damage may be missing/);
    assert.ok(chart.items.length > 0);
    for (const item of chart.items) {
      assert.equal(item.path[0].label, 'Building name unavailable'); assert.equal(item.path[1].label, 'State unknown');
      assert.doesNotMatch(item.note ?? '', /refus|absent|request/i, 'no backend reason in the hover'); assert.equal(typeof item.exact, 'bigint');
      assert.equal(item.unknownValue, true, 'incomplete intervals remain unknown');
    }
    assert.equal(chart.authoritativeTotal!.unknownValue, true);
    renderToStaticMarkup(createElement(Breakdown, {data: {...reply, parts: reply.parts.map(part => ({...part, state: 7}))}, runtime: {client: {}, clock: {}} as never,
      track: state.track as unknown as BoundTrack, tag: 'B', identity: '1:9', namedAt: null, from: 10, hour: 19, end: 20, dealt: false, options: {}, active: true}));
    assert.ok(host.__bombingBreakdown!.items.length > 0);
    for (const item of host.__bombingBreakdown!.items) assert.equal(item.path[1].label, 'State name not recorded', 'a state name the gate withholds is not recorded, never pending');
  } finally {await vite.close(); delete host.__airDrill; delete host.__bombingBreakdown;}
});

test('actual Air view opens the witnessed chooser, holds days, follows the clock, pins and rejects replaced owners', async () => {
  const state = fixture(), globals = globalThis as typeof globalThis & { __airDrill?: ReturnType<typeof fixture> };
  globals.__airDrill = state;
  const vite = await server({ ...common,
    '/dashboard/useMilitaryAirLedger.ts': 'export const useMilitaryAirLedger = () => ({ data: globalThis.__airDrill.data, track: globalThis.__airDrill.track, retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => globalThis.__airDrill.groups;',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#777";',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__airDrill.chart = props; return null; };',
    '/dashboard/AirLossDrill.tsx': 'export default props => { globalThis.__airDrill.detail = props; return null; };',
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__airDrill.picker = props; return null; };',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      const open = props.drill.open && props.subject !== false;
      globalThis.__airDrill.host = { ...props, open, onFollow: props.drill.follow, onPause: props.drill.pause, following: props.drill.following };
      return createElement('div', null, props.main, open ? props.children : null); };`,
  });
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const metadata = await vite.ssrLoadModule('/ui/src/widgets/air-kills-losses/definition.ts');
    registry.registerWidgets([metadata.definition]);
    const View = (await vite.ssrLoadModule('/ui/src/dashboard/MilitaryAirView.tsx')).default as ComponentType<WidgetProps & {
      family: 'airLosses'; fields: typeof OPTIONS; title: string;
    }>;
    const props = { widget: { id: 'widget', kind: 'air-kills-losses', title: 'Air', trackIds: ['track'],
      countries: { kind: 'group', group: { type: 'top', count: 2 }, membership: 'hour', aggregate: false }, time: { mode: 'follow' }, options: { view: 'lines', measure: 'dealt' } },
      runtime: { tracks: [state.track], client: {}, clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: state.playing }) },
        addWidgetBelow: (_id: string, widget: PinnedSpec) => state.added.push(persistedSpec(widget)) },
      active: true, onDiagnostics() {}, family: 'airLosses', fields: OPTIONS, title: 'Air kills & losses' } as unknown as WidgetProps & {
        family: 'airLosses'; fields: typeof OPTIONS; title: string;
      };
    const render = () => { state.cursor = 0; state.chart = undefined; state.host = undefined; state.picker = undefined; state.detail = undefined;
      return renderToStaticMarkup(createElement(View, props)); };
    const initialClient = props.runtime.client;
    render(); assert.equal(state.host!.open, false); assert.ok(state.chart!.onPick, 'the actual chart must expose Drill picking');
    const frame = plotFrame(400, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [10, 20], [-1, 4], false);
    const drawnLines = state.chart!.lines.map(line => ({ key: line.tag, runs: line.runs, held: true }));
    assert.deepEqual(pickLines(drawnLines, 12, { frame, until: 12, point: { x: frame.toX(12), y: frame.toY(2) } }), ['A']);
    const choices = pickLines(drawnLines, 12, { frame, until: 12, point: { x: frame.toX(12), y: frame.toY(0) } });
    assert.deepEqual(choices, ['A', 'B'], 'overlapping own/credited country lines use the shared drawn-line chooser');
    state.chart!.onPick!(12, undefined, choices); render();
    assert.deepEqual(state.picker!.allowedTags, choices); assert.deepEqual(state.picker!.flagAt, { branchId: 'branch', hour: 12 });
    assert.equal(state.host!.canFollow, false); assert.equal(state.host!.onPin, undefined);
    state.hour = -982; render(); assert.equal(state.picker!.flagAt!.hour, 12, 'the chooser keeps the drawn day while playback moves');
    const staleChooser = state.picker!.onChange;
    state.picker!.onChange('B', null); render();
    assert.equal(state.detail!.hour, 12); assert.equal(state.detail!.identity, '1:9');
    assert.equal(state.chart!.active, false, 'inline Drill suspends its inert main chart');
    state.detail!.onPick(14.9); render(); assert.equal(state.detail!.hour, 14);
    assert.equal(state.host!.onPin, undefined, 'no Pin is offered while the country’s identity is unknown');
    assert.equal(state.host!.pinRefused, 'Country identities are still loading.', 'the refused Pin says why');
    state.groups = { retry() {}, data: { countryIdentities: ['A', 'B', 'C'].map((tag, slot) => ({ tag, identities: [{ slot, countryId: 9, firstHour: 10, lastHour: 20 }] })) } };
    render();
    state.host!.onPin!();
    assert.deepEqual(state.added[0].time, { mode: 'pin', windows: { track: { firstOffset: 4, lastOffset: 4 } } });
    assert.equal(state.added[0].options!.show, 'breakdown');
    assert.equal(state.added[0].options!.holder, JSON.stringify(['campaign', 'branch', 'B', '1:9']));
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 18);
    state.hour = -981; render(); assert.equal(state.detail!.hour, 19, 'native history survives leaving top N');
    state.host!.onPause!(); render(); assert.equal(state.detail!.hour, 19);
    state.hour = -982; render(); assert.equal(state.detail!.hour, 19, 'pause holds the shown hour when playback seeks');
    state.host!.onPin!(); assert.deepEqual(state.added.at(-1)!.time, { mode: 'pin', windows: { track: { firstOffset: 9, lastOffset: 9 } } });
    state.added.pop();
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 18); state.hour = -981; render();
    const stableOwner = state.chart!.pickOwner, oldPick = state.chart!.onPick!, oldPin = state.host!.onPin!, oldFollow = state.host!.onFollow;
    state.track.source.revision++; state.data = { ...state.data, regions: [{ id: '40001', name: 'Reloaded content', missing: null }], warnings: ['content reload'] };
    render(); assert.equal(state.chart!.pickOwner, stableOwner); assert.equal(state.detail!.identity, '1:9');
    props.runtime.client = {} as typeof props.runtime.client;
    render(); assert.equal(state.host!.open, false);
    oldPick(12, 'B'); staleChooser('B', null); oldFollow(); oldPin(); render();
    assert.equal(state.host!.open, false, 'a replaced client cannot reopen an old actor');
    assert.equal(state.added.length, 1, 'a replaced client cannot pin an old actor');
    props.runtime.client = initialClient; render(); oldPick(12, 'B'); oldFollow(); oldPin(); render();
    assert.equal(state.host!.open, false, 'A → B → A cannot restore an earlier pick owner');
    assert.equal(state.added.length, 1);
    state.playing = true; state.chart!.onPick!(19, 'B'); render(); assert.equal(state.host!.following, true);
    state.detail!.onPick(13); render(); assert.equal(state.host!.following, false); assert.equal(state.detail!.hour, 13);
    props.widget.options = { ...props.widget.options, drill: 'popout' }; render();
    assert.equal(state.host!.mode, 'popout'); assert.equal(state.chart!.active, true);
    props.widget.time = { mode: 'campaign' }; render();
    state.playing = false; state.chart!.onPick!(12, undefined, ['B', 'A']); render(); state.picker!.onChange('B', null); render();
    assert.equal(state.detail!.hour, 12); assert.equal(state.host!.canFollow, true, 'Campaign still offers the actual shared clock');
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 19, 'Follow is independent of the parent campaign end');
    state.host!.onPin!(); assert.deepEqual(state.added.at(-1)!.time, { mode: 'follow' });
    state.hour = -979; render(); assert.equal(state.detail!.hour, 21, 'an out-of-branch clock is never replaced with campaign end');
    assert.equal(state.host!.canFollow, false); assert.equal(state.host!.onPin, undefined);
    state.hour = -981;
    props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 2, lastOffset: 2 } } }; render();
    state.chart!.onPick!(12, undefined, ['B', 'A']); render(); state.picker!.onChange('B', null); render();
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 19, 'a parent Pin does not pin the independent Follow clock');
    props.widget.time = { mode: 'range', firstOffset: 1, lastOffset: 10 }; render();
    state.chart!.onPick!(15, 'B'); render(); state.detail!.onPick(15); render(); state.host!.onPin!();
    assert.deepEqual(state.added.at(-1)!.time, { mode: 'range', firstOffset: 1, lastOffset: 5 });
    props.widget = state.added.at(-1)!; render(); assert.equal(state.host, undefined);
    assert.equal(state.detail!.hour, 15); assert.equal(state.detail!.from, 11); assert.equal(state.detail!.identity, '1:9');
    assert.match(render(), /Follow/, 'a restored range breakdown can follow the clock independently of its held upper bound');
    state.hour = -977;
    props.widget.time = { mode: 'follow' };
    assert.match(render(), /This time is outside the recording/); assert.equal(state.detail, undefined);
  } finally { await vite.close(); delete globals.__airDrill; }
});

test('actual cause producer keeps exact negative rows, positive geometry and native/publication hover dates', async () => {
  const state = fixture(), globals = globalThis as typeof globalThis & { __airDrill?: ReturnType<typeof fixture> };
  globals.__airDrill = state;
  const vite = await server({ ...common,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillParts.tsx': 'export default props => { globalThis.__airDrill.parts = props; return null; };',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__airDrill.history = props; return null; };',
    '/components/DateTimePicker.tsx': 'export default props => { globalThis.__airDrill.datePicker = props; return null; };',
  });
  try {
    const Drill = (await vite.ssrLoadModule('/ui/src/dashboard/AirLossDrill.tsx')).default as typeof AirLossDrill;
    const picked: number[] = [];
    const props = { index: militaryAirIndex(state.data), track: state.track,
      runtime: { client: {}, clock: {} }, tag: 'B', identity: '1:9', from: 10, hour: 16, end: 20, dealt: false,
      active: true, afterPlayhead: false, onPick: (at: number) => picked.push(at), pickOwner: {} } as unknown as ComponentProps<typeof AirLossDrill>;
    const findReset = (node: unknown): (() => void) | undefined => {
      if (Array.isArray(node)) return node.map(findReset).find(Boolean);
      if (!isValidElement<{ children?: unknown; onClick?: () => void }>(node)) return;
      return node.type === 'button' ? node.props.onClick : findReset(node.props.children);
    };
    const render = () => {
      state.cursor = 0; state.parts = undefined; state.history = undefined; state.datePicker = undefined;
      const element = Drill(props); state.reset = findReset(element);
      return renderToStaticMarkup(element);
    };
    render();
    const correction = state.parts!.parts.find(part => part.id === '3')!;
    assert.equal(correction.raw, -100000n); assert.equal(correction.text, '−1');
    assert.equal(state.parts!.positiveBars, true); assert.equal(state.parts!.signedShares, true); assert.ok(state.parts!.onPick);
    assert.equal(state.parts!.totalRow!.value, '9,007,199,254,740,992');
    assert.equal(state.parts!.parts.find(part => part.id === '1')!.text, '9,007,199,254,740,993');
    const extent = state.history!.layers[0].extent!([10, 20]); assert.equal(extent![0], 0, 'negative corrections have no positive area');
    assert.equal(state.history!.afterPlayhead, false); assert.ok(state.history!.overlay);
    const tip = (hour: number) => state.history!.tooltip!(hour, {} as never, []);
    assert.equal(tip(15)!.rows.find(row => row.label === 'Own losses')!.value, '0');
    assert.equal(tip(16)!.rows.find(row => row.label === 'Last supplied native event'), undefined);
    assert.equal(tip(16)!.rows.find(row => row.label === 'Event supplied'), undefined);
    assert.equal(tip(18), null, 'unreached future reads are hidden');
    state.parts!.onPick!('3'); assert.match(render(), /History: Air accidents/);
    assert.deepEqual(state.history!.layers[0].extent!([10, 20]), [0, 0]);
    const oldCause = state.parts!.onPick!, oldReset = state.reset!, oldDay = state.history!.onPick!;
    props.tag = 'A'; props.identity = '0:9';
    assert.match(render(), /All causes/);
    assert.deepEqual(state.parts!.parts.map(part => part.id), ['1'], 'the same popout gets the new native holder inventory');
    state.parts!.onPick!('1'); assert.match(render(), /History: Aircraft combat/);
    oldCause('1'); oldReset(); oldDay(14);
    assert.match(render(), /History: Aircraft combat/, 'old holder callbacks cannot overwrite or clear the current cause');
    assert.deepEqual(picked, []);
    props.tag = 'B'; props.identity = '1:9';
    assert.match(render(), /All causes/);
    oldCause('3'); assert.match(render(), /All causes/, 'A → B → A cannot revive the first holder owner');
    state.parts!.onPick!('3'); render();
    const clientCause = state.parts!.onPick!, clientReset = state.reset!, originalClient = props.runtime.client;
    props.runtime.client = {} as typeof props.runtime.client; render();
    props.runtime.client = originalClient; render(); clientCause('3'); clientReset();
    assert.match(render(), /All causes/, 'a client A → B → A requires a fresh cause owner');
    state.parts!.onPick!('3'); render();
    const inactiveCause = state.parts!.onPick!, inactiveReset = state.reset!;
    props.active = false; render(); inactiveCause('1'); inactiveReset(); props.active = true;
    assert.match(render(), /History: Air accidents/, 'inactive delayed controls cannot change the current cause');
    state.reset!(); render();
    props.hour = 18; props.afterPlayhead = true; render();
    assert.equal(state.parts!.showShares, false, 'a crossed missing read never yields complete shares');
    assert.equal(tip(17), null, 'a real observation gap is unavailable, not zero');
    props.hour = 17;
    assert.match(render(), /not recorded at this moment/);
    assert.equal(state.history!.playhead, 17); assert.deepEqual(state.history!.extent, [10, 20]);
    assert.equal(state.parts!.showShares, false);
    assert.ok(state.parts!.parts.every(part => part.raw === null && part.value === null && part.text === 'Not recorded'));
    state.history!.onPick!(16.9); assert.equal(picked.at(-1), 16);
    props.hour = picked.at(-1)!; assert.doesNotMatch(render(), /not recorded at this moment/);
    props.end = 16;
    props.hour = 18;
    assert.match(render(), /not recorded at this moment/, 'following beyond a selected range never changes its upper bound');
    assert.equal(state.history!.playhead, 18); assert.deepEqual(state.history!.extent, [10, 16]);
    assert.deepEqual([state.datePicker!.value, state.datePicker!.min, state.datePicker!.max], [18, 10, 16]);
    state.history!.onPick!(14); props.hour = picked.at(-1)!;
    assert.doesNotMatch(render(), /not recorded at this moment/, 'a standalone range can pick a recorded day without closing');
    props.hour = 9; props.afterPlayhead = false; assert.match(render(), /not recorded at this moment/);
    assert.equal(state.history!.layers[0].extent!([10, 16]), null, 'a clock before the range does not disclose later reads');
    state.datePicker!.onChange(14); props.hour = picked.at(-1)!;
    assert.doesNotMatch(render(), /not recorded at this moment/, 'the shared date picker recovers when no earlier geometry can be drawn');
    props.end = 20;
    props.hour = 21;
    assert.match(render(), /not recorded at this moment/);
    assert.equal(state.history!.playhead, 21); assert.deepEqual(state.history!.extent, [10, 20]);
  } finally { await vite.close(); delete globals.__airDrill; }
});

const bombingData = (): MilitaryAirLedger => ({ ...data(), family: 'bombing', parts: data().parts.map((part, i) => ({ ...part,
  opponentKnown: null, cause: 1, buildingToken: i === 1 ? 'correction' : 'factory', buildingId: i === 1 ? 2 : 1, state: 7 })),
  buildings: [{ id: 'factory', name: 'Factory', missing: null }, { id: 'correction', name: 'Corrected building', missing: null }] });

test('bombing parts retain exact corrections, selectable positive history and controls at unavailable dates', async () => {
  const state = fixture(), globals = globalThis as typeof globalThis & { __airDrill?: ReturnType<typeof fixture> };
  state.data = validateMilitaryAirReply(bombingData()); globals.__airDrill = state;
  const vite = await server({ ...common,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillParts.tsx': 'export default props => { globalThis.__airDrill.parts = props; return null; };',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__airDrill.history = props; return null; };',
    '/components/DateTimePicker.tsx': 'export default props => { globalThis.__airDrill.datePicker = props; return null; };',
  });
  try {
    const Drill = (await vite.ssrLoadModule('/ui/src/dashboard/BombingDrill.tsx')).default as typeof BombingDrill;
    const picked: number[] = [];
    const props = { data: state.data, track: state.track, runtime: { client: {}, clock: {} }, tag: 'B', identity: '1:9',
      from: 10, hour: 16, end: 20, dealt: false, active: true, pickOwner: {}, onPick: (at: number) => picked.push(at) } as unknown as ComponentProps<typeof BombingDrill>;
    const render = () => { state.cursor = 0; state.parts = undefined; state.history = undefined; state.datePicker = undefined;
      return renderToStaticMarkup(createElement(Drill, props)); };
    render();
    const correction = state.parts!.parts.find(part => part.id === 'correction')!;
    assert.equal(correction.raw, -100000n); assert.equal(correction.text, '−0.01');
    assert.equal(state.parts!.parts.find(part => part.id === 'factory')!.text, '90,071,992,547,409.93');
    assert.equal(state.parts!.positiveBars, true); assert.equal(state.parts!.signedShares, true);
    state.parts!.onPick!('correction'); assert.match(render(), /History: Corrected building/);
    assert.deepEqual(state.history!.layers[0].extent!([10, 20]), [0, 0]);
    assert.equal(state.history!.afterPlayhead, false); assert.ok(state.history!.overlay);
    assert.equal(state.history!.tooltip!(18, {} as never, []), null);
    const oldPart = state.parts!.onPick!, oldDay = state.history!.onPick!;
    props.tag = 'A'; props.identity = '0:9'; render();
    oldPart('correction'); oldDay(14); assert.match(render(), /All building types/); assert.deepEqual(picked, []);
    props.tag = 'B'; props.identity = '1:9'; render(); oldPart('correction'); assert.match(render(), /All building types/);
    props.hour = 17; assert.match(render(), /not recorded at this moment/);
    assert.ok(state.parts!.parts.every(part => part.raw === null)); assert.equal(state.parts!.showShares, false);
    assert.deepEqual(state.history!.extent, [10, 20]);
    state.datePicker!.onChange(16.9); assert.deepEqual(picked, [16]);
    props.hour = 9; render(); assert.equal(state.history!.layers[0].extent!([10, 20]), null);
    props.hour = 16;
    props.data = { ...state.data, buildings: [
      { id: 'factory', name: null, missing: 'Synthetic absent localization' },
      { id: 'correction', name: '', missing: null },
    ] };
    render();
    assert.equal(state.parts!.parts.find(part => part.id === 'factory')!.label, 'Building name unavailable');
    assert.equal(state.parts!.parts.find(part => part.id === 'correction')!.label, '');
  } finally { await vite.close(); delete globals.__airDrill; }
});

test('bombing Drill follows the shared clock from a campaign view and rejects replaced client controls', async () => {
  const state = fixture(), globals = globalThis as typeof globalThis & { __airDrill?: ReturnType<typeof fixture> };
  state.data = validateMilitaryAirReply(bombingData()); globals.__airDrill = state;
  state.groups = { retry() {}, data: { branchId: 'branch', warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [], humans: [],
    unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: ['A', 'B', 'C'].map((tag, slot) => ({ tag, identities: [{ slot, countryId: 9, firstHour: 10, lastHour: 20 }] })) } };
  const vite = await server({ ...common,
    '/dashboard/useMilitaryAirLedger.ts': 'export const useMilitaryAirLedger = () => ({ data: globalThis.__airDrill.data, track: globalThis.__airDrill.track, retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => globalThis.__airDrill.groups;',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#777";',
    '/charts/butterfly/index.ts': 'export const ButterflyChart = props => { globalThis.__airDrill.butterfly = props; return null; };',
    '/dashboard/BombingDrill.tsx': 'export default props => { globalThis.__airDrill.bombing = props; return null; };',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      const open = props.drill.open && props.subject !== false;
      globalThis.__airDrill.host = { ...props, open, onFollow: props.drill.follow, onPause: props.drill.pause, following: props.drill.following };
      return createElement('div', null, props.main, open ? props.children : null); };`,
  });
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const metadata = await vite.ssrLoadModule('/ui/src/widgets/bombing-damage/definition.ts');
    const breakdown = await vite.ssrLoadModule('/ui/src/widgets/bombing-damage-breakdown/definition.ts');
    registry.registerWidgets([metadata.definition, breakdown.definition]);
    const View = (await vite.ssrLoadModule('/ui/src/dashboard/MilitaryAirView.tsx')).default;
    const props = { widget: { id: 'bombing', kind: 'bombing-damage', trackIds: ['track'], countries: { kind: 'all' }, time: { mode: 'campaign' } },
      runtime: { tracks: [state.track], client: {}, clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: state.playing }) },
        addWidgetBelow: (_id: string, widget: PinnedSpec) => state.added.push(persistedSpec(widget)) }, active: true, onDiagnostics() {},
      family: 'bombing', fields: metadata.OPTIONS, title: 'Bombing damage' };
    const render = () => { state.cursor = 0; state.bombing = undefined; return renderToStaticMarkup(createElement(View, props)); };
    render(); state.butterfly!.onSelect(state.butterfly!.items[0], 'left'); render();
    assert.equal(state.bombing!.hour, 20); assert.equal(state.host!.canFollow, true); assert.equal(state.butterfly!.active, false);
    state.hour = -981; state.host!.onFollow(); render(); assert.equal(state.bombing!.hour, 19);
    state.host!.onPin!(); assert.deepEqual(state.added[0].time, { mode: 'follow' });
    state.host!.onPause!(); render(); const pausedHour = state.bombing!.hour;
    state.hour++; render(); assert.equal(state.bombing!.hour, pausedHour, 'bombing pause keeps its displayed moment');
    state.host!.onFollow(); render(); assert.equal(state.bombing!.hour, pausedHour + 1);
    state.hour--; render();
    state.bombing!.onPick(16); render(); assert.equal(state.bombing!.hour, 16); assert.equal(state.host!.following, false);
    const oldPick = state.bombing!.onPick, oldPin = state.host!.onPin!, oldFollow = state.host!.onFollow;
    const oldClient = props.runtime.client; props.runtime.client = {}; render(); assert.equal(state.host!.open, false);
    oldPick(14); oldPin(); oldFollow(); render(); assert.equal(state.host!.open, false); assert.equal(state.added.length, 1);
    props.runtime.client = oldClient; render(); oldPin(); oldFollow(); render(); assert.equal(state.host!.open, false);
    props.widget.time.mode = 'follow'; state.hour = -984; state.playing = true; render();
    state.butterfly!.onSelect(state.butterfly!.items[0], 'left'); render();
    state.hour = -983; render(); assert.equal(state.bombing!.hour, 17, 'a missing live read keeps the breakdown mounted');
  } finally { await vite.close(); delete globals.__airDrill; }
});

test('bombing sunburst uses content titles and hover keys while exact recorded damage remains independent', async () => {
  const state: { data?: { states: { stateId: number; name: string | null; missing: string | null }[] }; error?: Error;
    sunburst?: import('../charts/sunburst/Sunburst').SunburstProps } = { data: { states: [{ stateId: 7, name: 'Dated mod state', missing: null }] } };
  const globals = globalThis as typeof globalThis & { __bombingTitles?: typeof state };
  globals.__bombingTitles = state;
  const vite = await server({ ...common,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/dashboard/products.ts': 'export const useProduct = () => globalThis.__bombingTitles; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/charts/sunburst/Sunburst.tsx': 'export default props => { globalThis.__bombingTitles.sunburst = props; return null; };',
  });
  try {
    const View = (await vite.ssrLoadModule('/ui/src/widgets/bombing-damage-breakdown/BombingBreakdown.tsx')).default;
    const data = validateMilitaryAirReply(bombingData());
    const props = { data, track: fixture().track, runtime: { client: {}, clock: {} }, tag: 'B', identity: '1:9',
      from: 10, hour: 16, end: 20, dealt: false, active: true, options: {} };
    const render = () => renderToStaticMarkup(createElement(View, props));
    render();
    const parts = () => state.sunburst!.items;
    const factory = () => parts().find(item => item.path[0].id === 'building:factory')!;
    assert.equal(factory().path[0].label, 'Factory');
    assert.equal(factory().path[1].label, 'Dated mod state');
    const exact = factory().exact;
    props.data = { ...data, buildings: [{ id: 'factory', name: null, missing: 'Synthetic absent localization' },
      { id: 'correction', name: '', missing: null }] };
    render();
    assert.equal(factory().path[0].label, 'Building name unavailable');
    assert.equal(factory().exact, exact, 'missing content text does not change recorded damage');
    assert.match(factory().note!, /Key: factory/);
    assert.doesNotMatch(factory().note!, /Synthetic absent localization/, 'the lookup key, not the localization diagnostic');
    assert.equal(parts().find(item => item.path[0].id === 'building:correction')!.path[0].label, '');
    props.data = { ...data, buildings: [{ id: 'factory', name: 'factory', missing: null }] };
    render();
    assert.equal(factory().path[0].label, 'factory', 'an authored title identical to its key remains authoritative');
    state.data = { states: [{ stateId: 7, name: '', missing: null }] };
    render();
    assert.equal(factory().path[1].label, '');
    state.data = undefined;
    render();
    assert.equal(factory().path[1].label, 'Loading name…', 'a state name still loading is not called unrecorded');
    assert.equal(factory().exact, exact);
  } finally { await vite.close(); delete globals.__bombingTitles; }
});

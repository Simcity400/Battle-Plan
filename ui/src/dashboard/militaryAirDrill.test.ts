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
import type { WidgetSpec } from './model';
import { persistedSpec, type PinnedSpec } from './regionTime.ts';
import type AirLossDrill from './AirLossDrill.tsx';
import { militaryAirIndex } from './militaryAirLedger.ts';
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
  return { hooks: [] as unknown[], cursor: 0, data: data(),
    track: { id: 'track', axisOffset: 1000, source: { id: 'source', revision: 1, metadata: { countries: [], campaignKey: 'campaign' } },
      branch: { id: 'branch', firstHour: 10, lastHour: 20, gaps: [span(17)] } }, hour: -988, playing: false, added: [] as WidgetSpec[],
    groups: { retry() {} } as { retry(): void; data?: unknown },
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
        if (path.endsWith('/dashboard/MilitaryAirView.tsx') || path.endsWith('/dashboard/AirLossDrill.tsx')
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

test('actual Air view opens the witnessed chooser, holds days, follows the clock and rejects replaced owners', async () => {
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
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 18);
    state.hour = -981; render(); assert.equal(state.detail!.hour, 19, 'native history survives leaving top N');
    state.host!.onPause!(); render(); assert.equal(state.detail!.hour, 19);
    state.hour = -982; render(); assert.equal(state.detail!.hour, 19, 'pause holds the shown hour when playback seeks');
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 18); state.hour = -981; render();
    const stableOwner = state.chart!.pickOwner, oldPick = state.chart!.onPick!, oldFollow = state.host!.onFollow;
    state.track.source.revision++; state.data = { ...state.data, regions: [{ id: '40001', name: 'Reloaded content', missing: null }], warnings: ['content reload'] };
    render(); assert.equal(state.chart!.pickOwner, stableOwner); assert.equal(state.detail!.identity, '1:9');
    props.runtime.client = {} as typeof props.runtime.client;
    render(); assert.equal(state.host!.open, false);
    oldPick(12, 'B'); staleChooser('B', null); oldFollow(); render();
    assert.equal(state.host!.open, false, 'a replaced client cannot reopen an old actor');
    props.runtime.client = initialClient; render(); oldPick(12, 'B'); oldFollow(); render();
    assert.equal(state.host!.open, false, 'A → B → A cannot restore an earlier pick owner');
    state.playing = true; state.chart!.onPick!(19, 'B'); render(); assert.equal(state.host!.following, true);
    state.detail!.onPick(13); render(); assert.equal(state.host!.following, false); assert.equal(state.detail!.hour, 13);
    props.widget.options = { ...props.widget.options, drill: 'popout' }; render();
    assert.equal(state.host!.mode, 'popout'); assert.equal(state.chart!.active, true);
    props.widget.time = { mode: 'campaign' }; render();
    state.playing = false; state.chart!.onPick!(12, undefined, ['B', 'A']); render(); state.picker!.onChange('B', null); render();
    assert.equal(state.detail!.hour, 12); assert.equal(state.host!.canFollow, true, 'Campaign still offers the actual shared clock');
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 19, 'Follow is independent of the parent campaign end');
    state.hour = -979; render(); assert.equal(state.detail!.hour, 21, 'an out-of-branch clock is never replaced with campaign end');
    assert.equal(state.host!.canFollow, false); assert.equal(state.host!.onPin, undefined);
    state.hour = -981;
    props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 2, lastOffset: 2 } } }; render();
    state.chart!.onPick!(12, undefined, ['B', 'A']); render(); state.picker!.onChange('B', null); render();
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 19, 'a parent Pin does not pin the independent Follow clock');
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



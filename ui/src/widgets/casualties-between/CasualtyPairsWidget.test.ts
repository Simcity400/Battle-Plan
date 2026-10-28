import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type DrillHost from '../../charts/drill/DrillHost';
import type ChordChart from '../../charts/chord/ChordChart';
import type Table from '../../charts/table/Table';
import type { DrillAction, DrillState } from '../../charts/drill/drillState';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { TimelineProps } from '../../charts/timeline/Timeline';
import type { UnitLeadersData } from '../generals/generalsData';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { createWidget } from '../../dashboard/model.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { CasualtyPairsReply, CountryGroupReply } from '../../generated/dashboard';
import { casualtyPairsAt } from './casualtyPairsData.ts';
import '../definitions.ts';

type HostProps = Parameters<typeof DrillHost>[0];
type ChordProps = Parameters<typeof ChordChart>[0];
type TableProps = Parameters<typeof Table>[0];
type State = { hour: number; drill: DrillState; slots: unknown[]; cursor: number; host?: HostProps; chord?: ChordProps;
  chart?: TimeChartProps; table?: TableProps; parts?: number; subscriptions: boolean[]; data: unknown; windows: Map<number, unknown>;
  groups?: CountryGroupReply; groupsError?: string; groupActive?: boolean; groupRetries?: number; windowIdentity?: string | null;
  windowRequests?: { params: { tags: string[]; between: boolean; window: number } }[];
  career?: { hour: number; widget: WidgetSpec; onPick: (hour: number) => void }; timeline?: TimelineProps; actions?: { label: string; run: () => void }[] };

async function fixture(state: State, module: string, realCareer = false) {
  const globals = globalThis as typeof globalThis & { __landDrill?: State;
    __landReducer?: (reducer: typeof reduceDrill) => [DrillState, (action: DrillAction) => void];
    __landState?: (initial: unknown) => [unknown, (value: unknown) => void];
    __landMemo?: (factory: () => unknown, deps: readonly unknown[]) => unknown;
    __landRef?: (initial: unknown) => { current: unknown } };
  globals.__landDrill = state;
  globals.__landReducer = reducer => [state.drill, action => { state.drill = reducer(state.drill, action); }];
  globals.__landState = initial => {
    const index = state.cursor++;
    if (!(index in state.slots)) state.slots[index] = initial;
    return [state.slots[index], value => { state.slots[index] = typeof value === 'function'
      ? (value as (old: unknown) => unknown)(state.slots[index]) : value; }];
  };
  const memos: { deps: readonly unknown[]; value: unknown }[] = [];
  const refs: { current: unknown }[] = [];
  let memoCursor = 0, refCursor = 0;
  globals.__landMemo = (factory, deps) => {
    const index = memoCursor++, before = memos[index];
    if (before && before.deps.length === deps.length && before.deps.every((value, i) => Object.is(value, deps[i]))) return before.value;
    const value = factory(); memos[index] = { deps: [...deps], value }; return value;
  };
  globals.__landRef = initial => {
    const index = refCursor++;
    return refs[index] ?? (refs[index] = { current: initial });
  };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__landDrill.data, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, identity, request, wanted) => {
      const state = globalThis.__landDrill; state.windowIdentity = identity;
      state.windowRequests = identity ? wanted.map(request) : [];
      return { data: identity ? state.windows : new Map(), errors: new Map(), retry() {} };
    };`,
    '/dashboard/playback.ts': `export const usePlaybackHour = (_clock, active) => { globalThis.__landDrill.subscriptions.push(active); return globalThis.__landDrill.hour; };
      export const usePlaybackSpeed = () => 0;`,
    '/charts/table/useTableHour.ts': `export const useTableHour = (_clock, active) => { globalThis.__landDrill.subscriptions.push(active); return globalThis.__landDrill.hour; };`,
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active = true) => {
      const state = globalThis.__landDrill; state.groupActive = active;
      return { data: active ? state.groups : undefined, error: state.groupsError, retry() { state.groupRetries = (state.groupRetries ?? 0) + 1; } };
    };`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: hour => tag => ({ name: tag + ' at ' + hour, flagPending: true }) });
      export const flagReadSpec = () => ({}); export const flagReadMetadata = () => ({});`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
      export const useCountryFactsAt = () => new Map(); export const useCountryFactsRead = () => new Map(); export const useCountryNamesAt = () => new Map();`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456', '#654321'], muted: '#777777', surface: '#000000', countryColours: 'raw' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open(_x, _y, actions) { globalThis.__landDrill.actions = actions; } });',
    '/charts/drill/DrillHost.tsx': 'export default props => { globalThis.__landDrill.host = props; return props.open ? props.children : props.main; };',
    '/charts/drill/DrillParts.tsx': 'export default () => {globalThis.__landDrill.parts++; return null;};',
    '/charts/chord/ChordChart.tsx': 'export default props => { globalThis.__landDrill.chord = props; return null; };',
    '/charts/sunburst/Sunburst.tsx': 'export default () => null;',
    '/charts/table/Table.tsx': `export const CountryCell = () => null;
      export default props => { globalThis.__landDrill.table = props; return null; };`,
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__landDrill.chart = props; return null; };',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__landDrill.timeline = props; return null; };',
    '/widgets/generals/useLeaders.ts': `export const useLeaderCountries = () => ({ countries: null, group: {} });
      export const useLeaders = () => ({ data: globalThis.__landDrill.data });`,
    '/widgets/generals/portraits.ts': 'export const useLeaderPortraits = () => ({ at() {}, warnings: [] });',
    '/widgets/general-career/CareerView.tsx': 'export default props => { globalThis.__landDrill.career = props; return null; };',
  };
  if (realCareer) delete replacements['/widgets/general-career/CareerView.tsx'];
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'land-drill-actual-caller', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith(module)) return source.replace(/\buseReducer\(/g, 'globalThis.__landReducer(')
          .replace(/\buseState(?:<[^;\n]*>)?\(/g, 'globalThis.__landState(')
          .replace(/\buseMemo(?:<[^;\n]*>)?\(/g, 'globalThis.__landMemo(')
          .replace(/\buseRef(?:<[^;\n]*>)?\(/g, 'globalThis.__landRef(');
      },
    }] });
  await vite.ssrLoadModule('/ui/src/widgets/definitions.ts');
  const { default: Widget } = await vite.ssrLoadModule(`/ui/src${module}`);
  const track = { id: 'main', axisOffset: 30, branch: { id: '1', firstHour: 100, lastHour: 160 },
    source: { id: 'source', revision: '1', metadata: { campaignKey: 'synthetic', countries: [] } } };
  const pinned: WidgetSpec[] = [];
  const runtime = { tracks: [track], client: {}, clock: { getHour: () => state.hour, positionAt: () => state.hour,
    getSnapshot: () => ({ position: state.hour, playing: true, seekSequence: 0 }) }, addWidgetBelow: (_id: string, widget: WidgetSpec) => pinned.push(widget) };
  const render = (widget: WidgetSpec) => {
    state.cursor = 0; memoCursor = 0; refCursor = 0;
    state.parts = 0;
    state.chart = undefined; state.host = undefined; state.chord = undefined; state.table = undefined; state.timeline = undefined; state.career = undefined; state.subscriptions = [];
    return renderToStaticMarkup(createElement(Widget, { widget, runtime, active: true, onDiagnostics() {} } as unknown as WidgetProps));
  };
  return { render, track, runtime, pinned, async close() {
    await vite.close(); delete globals.__landDrill; delete globals.__landReducer; delete globals.__landState; delete globals.__landMemo; delete globals.__landRef;
  } };
}

test('actual manual pair choices load their catalogue and request original native roots without an all-world window', async () => {
  const span = { firstHour: 100, lastHour: 160 };
  const reply: CasualtyPairsReply = { branchId: '1', available: true, windows: [span], window: 0, observed: [span], hasGap: false,
    warnings: [], pairs: [{ identity: '1:10|2:20', firstCountry: 'Y', secondCountry: 'enemy', runs: [{ firstHour: 120, lastHour: 160,
      readHour: 120, hourly: false, firstTaken: '0', secondTaken: '-12', incomplete: false }] }] };
  const state: State = { hour: 95, drill: CLOSED, slots: [], cursor: 0, subscriptions: [],
    data: { ...reply, window: null, observed: [], pairs: [] }, windows: new Map([[0, reply]]) };
  const view = await fixture(state, '/widgets/casualties-between/CasualtyPairsWidget.tsx');
  const requests = (): State['windowRequests'] => state.windowRequests;
  try {
    const widget = { ...createWidget('casualties-between', 'main'), countries: { kind: 'tags' as const, tags: ['X'] }, time: { mode: 'follow' as const } };
    assert.match(view.render(widget), /Loading casualty pairs/);
    assert.equal(state.groupActive, true); assert.equal(state.windowIdentity, null); assert.deepEqual(state.windowRequests, []);
    state.groups = { branchId: '1', humans: [{ tag: 'Y', firstHour: 120, lastHour: 127 }], factions: [], wars: [], warStarts: [], capitulations: [],
      identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [
        { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }] },
        { tag: 'Y', identities: [{ firstHour: 100, lastHour: 119, slot: 3, countryId: 99 }, { firstHour: 120, lastHour: 160, slot: 1, countryId: 10 }] },
        { tag: 'enemy', identities: [{ ...span, slot: 2, countryId: 20 }] },
      ] };
    view.render(widget);
    assert.ok(state.windowIdentity); assert.deepEqual(requests()!.map(request => request.params.tags), [['X']]);
    assert.equal(requests()![0].params.between, false);
    const frame = state.chord!.frameAt!(0).data;
    assert.ok(frame, 'the backend-selected renamed native pair remains usable');
    const all = state.host!.main as unknown as { props: { children: { props?: { children?: string; onClick?: () => void } }[] } };
    all.props.children.find(child => child?.props?.children === 'All pairs')!.props!.onClick!();
    view.render(widget);
    assert.equal((state.table!.rows[0] as { value: bigint }).value, -12n);
    assert.equal((state.table!.rows[0] as { from: string }).from, 'Y');
    assert.equal((state.table!.rows[0] as { readHour: number }).readHour, 120, 'carried rows retain their supplying read');
    state.drill = CLOSED;
    view.render({ ...widget, options: { between: 'between' } });
    assert.equal(requests()![0].params.between, true, 'the backend retains its exact endpoint identity restriction');
    const grouped = { ...widget, countries: { kind: 'group' as const, group: { type: 'humans' as const }, membership: 'hour' as const, aggregate: false } };
    view.render(grouped); assert.ok(state.chord!.frameAt!(0).data, 'the group passes its own membership hour');
    state.hour = 100;
    view.render(grouped); assert.equal(state.chord, undefined, 'the group does not acquire manual all-ever membership');
    state.groupsError = 'Native catalogue unavailable';
    const failure = view.render(widget);
    assert.match(failure, /Couldn’t load\./);
    assert.match(failure, /Retry/);
    assert.doesNotMatch(failure.replace(/<[^>]*>/g, ' '), /Native catalogue unavailable/);
    state.groupsError = undefined;
    const missing = { ...widget, countries: { kind: 'tags' as const, tags: ['unknown'] } };
    assert.match(view.render(missing), /Selected countries are unavailable in this campaign\./);
    assert.equal(state.windowIdentity, null); assert.deepEqual(state.windowRequests, []);
    view.render({ ...widget, countries: { kind: 'all' as const } });
    assert.equal(state.groupActive, false); assert.deepEqual(requests()![0].params.tags, []);
  } finally { await view.close(); }
});

test('actual pair Drill picks dates, follows the offset clock and pins exact signed native rows', async () => {
  const windows = [{ firstHour: 100, lastHour: 119 }, { firstHour: 120, lastHour: 139 }, { firstHour: 140, lastHour: 160 }];
  const reply = (index: number, identity: string, amount: string): CasualtyPairsReply => ({ branchId: '1', available: true,
    windows, window: index, observed: [windows[index]], hasGap: false, warnings: [], pairs: [{ identity,
      firstCountry: 'mod attacker', secondCountry: 'mod victim', runs: [{ ...windows[index], readHour: windows[index].firstHour,
        hourly: true, firstTaken: '0', secondTaken: amount, incomplete: false }] }] });
  const first = reply(0, '1:10|2:20', '-12'), next = reply(1, '1:10|2:20', '3'), replaced = reply(2, '1:11|2:20', '999');
  next.pairs[0].firstCountry = 'renamed mod attacker';
  const state: State = { hour: 80, drill: CLOSED, slots: [], cursor: 0, subscriptions: [],
    data: { ...first, window: null, observed: [], pairs: [] }, windows: new Map([[0, first], [1, next], [2, replaced]]) };
  const view = await fixture(state, '/widgets/casualties-between/CasualtyPairsWidget.tsx');
  try {
    const widget = { ...createWidget('casualties-between', 'main'), time: { mode: 'follow' as const } };
    view.render(widget);
    const frame = state.chord!.frameAt!(0).data!;
    const id = casualtyPairsAt(first, 110).flows[0].id;
    // The all-pairs button reaches corrections even though their ribbons have no geometry.
    const all = state.host!.main as unknown as { props: { children: { props?: { children?: string; onClick?: () => void } }[] } };
    const button = all.props.children.find(child => child?.props?.children === 'All pairs');
    assert.ok(all.props.children.find(child => child?.props?.children === 'Corrections')?.props?.onClick,
      'the dedicated Corrections control stays available');
    assert.ok(button?.props?.onClick);
    assert.ok(frame, 'the current zero/correction frame remains available');
    button.props.onClick();
    const details = view.render(widget);
    assert.match(details, /Signed total: <b>-12<\/b>/);
    assert.equal(state.parts, 0, 'the same pairs are presented only in the table');
    assert.deepEqual(state.table!.columns.map(column => column.key), ['from', 'to', 'value', 'share']);
    assert.equal(state.table!.columns.find(column => column.key === 'share')!.value(state.table!.rows[0]), null,
      'corrections do not supply a positive share');
    assert.equal(state.host!.open, true);
    assert.equal(state.host!.canFollow, true);
    assert.ok(state.chart!.onPick, 'same-parts history supplies a date picker');
    state.chart!.onPick!(103.9);
    state.hour = 100;
    view.render(widget);
    assert.equal(state.host!.following, false);
    assert.equal(state.host!.day, formatDateTime(103));
    assert.equal((state.table!.rows[0] as { value: bigint }).value, -12n);
    state.host!.onPin!();
    const held = view.pinned.at(-1)!;
    assert.deepEqual(held.time, { mode: 'pin', windows: { main: { firstOffset: 3, lastOffset: 3 } } });
    assert.equal(held.options?.show, 'pairs');
    state.host!.onFollow();
    view.render(widget);
    assert.equal(state.host!.following, true);
    assert.equal(state.host!.day, formatDateTime(130));
    assert.equal(state.chart!.playhead, 130);
    assert.equal((state.table!.rows[0] as { id: string; value: bigint }).id, id);
    assert.equal((state.table!.rows[0] as { from: string }).from, 'renamed mod attacker');
    assert.equal((state.table!.rows[0] as { value: bigint }).value, 3n);
    const share = state.table!.columns.find(column => column.key === 'share')!;
    assert.equal(share.value(state.table!.rows[0]), 100);
    assert.equal(share.render!(state.table!.rows[0]), '100%');
    const amount = state.table!.columns.find(column => column.key === 'value')!;
    assert.match(renderToStaticMarkup(createElement(() => amount.render!(state.table!.rows[0]))), /casualty-pair-swatch/);
    const completeWindows = state.windows;
    state.windows = new Map([[0, first], [1, { ...next, pairs: next.pairs.map(pair => ({ ...pair,
      runs: pair.runs.map(run => ({ ...run, incomplete: true })) })) }], [2, replaced]]);
    const subtotal = view.render(widget);
    assert.match(subtotal, /Recorded subtotal: <b>3<\/b>/);
    assert.equal(state.table!.columns.find(column => column.key === 'share')!.value(state.table!.rows[0]), null);
    state.windows = completeWindows;
    for (const clock of [50, 140]) {
      state.hour = clock;
      const outside = view.render(widget);
      assert.match(outside, /outside the recording/);
      assert.doesNotMatch(outside, /Loading this moment/);
      assert.equal(state.host!.day, formatDateTime(clock + 30), 'Follow retains the actual clock date outside either branch boundary');
      assert.equal(state.host!.open, true);
      assert.equal(state.chart!.playhead, clock + 30);
      assert.equal(state.table, undefined);
      state.chart!.onPick!(103.9);
      view.render(widget);
      assert.equal(state.host!.day, formatDateTime(103));
      assert.equal((state.table!.rows[0] as { value: bigint }).value, -12n, 'history remains selectable while the parent Follow date is outside');
      state.host!.onFollow();
    }
    state.hour = 120;
    view.render(widget);
    assert.equal(state.table, undefined, 'replacement holders are absent from the selected raw cohort');
    state.host!.onPin!();
    assert.deepEqual(view.pinned.at(-1)!.time, { mode: 'follow' });
    state.drill = CLOSED; state.slots = [];
    const pinnedMarkup = view.render(held);
    assert.match(pinnedMarkup, /Follow playback/);
    assert.equal(state.chart!.playhead, null);
    assert.equal((state.table!.rows[0] as { value: bigint }).value, -12n);
    state.slots = []; state.hour = 140;
    const mainOutside = view.render(widget);
    assert.equal(state.host!.open, false);
    assert.match(mainOutside, /outside the recording/);
    assert.doesNotMatch(mainOutside, /Loading casualty pairs/);
  } finally { await view.close(); }
});

test('actual casualty breakdown holds Campaign and Pin dates until explicit Follow', async () => {
  const span = { firstHour: 100, lastHour: 160 };
  const reply = { branchId: '1', tag: 'mod actor', available: true, windows: [span], window: 0, observed: [span], hasGap: false,
    warnings: [], countries: [{ ...span, identity: '1:10', incomplete: false }], series: [], wars: [] };
  const state: State = { hour: 80, drill: CLOSED, slots: [], cursor: 0, subscriptions: [],
    data: { ...reply, window: null, observed: [], countries: [] }, windows: new Map([[0, reply]]) };
  const view = await fixture(state, '/widgets/casualties-breakdown/CasualtiesBreakdownWidget.tsx');
  try {
    for (const time of [{ mode: 'campaign' as const }, { mode: 'pin' as const, windows: { main: { firstOffset: 12, lastOffset: 12 } } }]) {
      state.hour = 80;
      const widget = { ...createWidget('casualties-breakdown', 'main'), countries: { kind: 'tags' as const, tags: ['mod actor'] }, time };
      state.slots = [{ breakdown: 'details', follow: false, hour: 112 }];
      view.render(widget);
      assert.equal(state.host!.day, formatDateTime(112));
      assert.equal(state.host!.canFollow, true);
      state.host!.onFollow();
      view.render(widget);
      assert.equal(state.host!.day, formatDateTime(110));
      assert.equal(state.chart!.playhead, 110);
      assert.deepEqual(state.subscriptions, [true]);
      state.host!.onPin!();
      assert.deepEqual(view.pinned.at(-1)!.time, { mode: 'follow' });
      state.chart!.onPick!(105);
      state.hour = 100;
      view.render(widget);
      assert.equal(state.host!.day, formatDateTime(105));
      assert.equal(state.host!.following, false);
    }
    state.slots = [];
    const pinned = { ...createWidget('casualties-breakdown', 'main'), countries: { kind: 'tags' as const, tags: ['mod actor'] },
      options: { show: 'details' }, time: { mode: 'pin' as const, windows: { main: { firstOffset: 12, lastOffset: 12 } } } };
    assert.match(view.render(pinned), /Follow playback/);
    assert.equal(state.chart!.playhead, null);
    const following = { ...pinned, options: {}, time: { mode: 'follow' as const } };
    for (const clock of [50, 140]) {
      state.hour = clock;
      state.slots = [{ breakdown: 'details', follow: true, hour: null }];
      const outside = view.render(following);
      assert.match(outside, /outside the recording/);
      assert.doesNotMatch(outside, /Loading this moment|Loading casualty history/);
      assert.equal(state.host!.open, true, 'missing parent windows cannot discard an open Drill');
      assert.equal(state.host!.day, formatDateTime(clock + 30));
      assert.equal(state.chart!.playhead, clock + 30);
      state.chart!.onPick!(112.8);
      view.render(following);
      assert.equal(state.host!.day, formatDateTime(112));
      assert.equal(state.host!.following, false);
      assert.ok(state.table, 'the history picker restores a recorded date without clamping Follow');
      state.slots = [];
      const mainOutside = view.render(following);
      assert.equal(state.host!.open, false);
      assert.match(mainOutside, /outside the recording/);
      assert.doesNotMatch(mainOutside, /Loading casualty history/);
    }
  } finally { await view.close(); }
});

function leaderData(): UnitLeadersData {
  return { branchId: '1', firstHour: 100, warnings: [],
    people: [{ characterId: 9, token: 'mod_character', name: 'Synthetic commander', portraitHour: 100, portraitReference: '21474841193' }],
    roles: [{ owner: 'mod actor', category: 1, rank: 'Synthetic rank', kind: 1, maximumSkill: 12,
      attributeNames: ['Attack', 'Defence', 'Planning', 'Logistics'] }],
    traits: [{ token: 'mod_trait', name: 'Synthetic trait', icon: null, missing: null }],
    leaders: [{ reference: '21474841193', runs: [
      { firstHour: 100, lastHour: 131, value: { person: 0, role: 0, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [] } },
      { firstHour: 132, lastHour: 160, value: { person: 0, role: 0, skill: 4, components: [1, 2, 3, 4], progress: null, traits: [0] } },
    ], traitGains: [{ hour: 132, token: 0, date: 'gained' }], commands: [{ formation: '99', kind: 'army', name: 'Synthetic formation',
      firstHour: 110, lastHour: 160, nameReadHour: 110, nameUnknown: [] }],
    battleStarts: [132], battleStartReadHours: [132], unknownBattleStarts: [], battleWon: [true], battleResultHours: [135],
    battleResultReadHours: [135], partialFrom: null }],
    observed: [{ firstHour: 100, lastHour: 160, leaders: true, landBattles: true, navalBattles: true, landCommands: true, navalCommands: true }],
    resultsMissing: '', navyCommandsMissing: '' };
}

test('actual Generals Follow advances from a held leaderboard and pins a live career', async () => {
  const state: State = { hour: 80, drill: CLOSED, slots: [], cursor: 0,
    subscriptions: [], data: leaderData(), windows: new Map() };
  const view = await fixture(state, '/widgets/generals/GeneralsWidget.tsx');
  try {
    for (const time of [{ mode: 'campaign' as const }, { mode: 'pin' as const, windows: { main: { firstOffset: 12, lastOffset: 12 } } }]) {
      state.drill = CLOSED; state.slots = [];
      const widget = { ...createWidget('generals', 'main'), time };
      view.render(widget);
      state.table!.onRowClick!(state.table!.rows[0]);
      view.render(widget);
      state.career!.onPick(112);
      view.render(widget);
      assert.equal(state.host!.day, formatDateTime(112));
      assert.equal(state.host!.canFollow, true);
      state.host!.onFollow();
      state.hour = 90;
      view.render(widget);
      assert.equal(state.host!.day, formatDateTime(120));
      assert.equal(state.career!.hour, 120);
      assert.deepEqual(state.career!.widget.time, { mode: 'follow' }, 'the career axis cannot retain the leaderboard Pin cutoff');
      assert.deepEqual(state.subscriptions, [true]);
      state.host!.onPin!();
      assert.deepEqual(view.pinned.at(-1)!.time, { mode: 'follow' });
      state.hour = 100;
      view.render(widget);
      assert.equal(state.career!.hour, 130);
    }
  } finally { await view.close(); }
});

test('real career lanes and statistics retain a picked date after Follow from an earlier parent Pin or Range', async () => {
  const state: State = { hour: 100, drill: CLOSED, slots: [], cursor: 0,
    subscriptions: [], data: leaderData(), windows: new Map() };
  const view = await fixture(state, '/widgets/generals/GeneralsWidget.tsx', true);
  try {
    for (const time of [{ mode: 'pin' as const, windows: { main: { firstOffset: 12, lastOffset: 12 } } },
      { mode: 'range' as const, firstOffset: 0, lastOffset: 12 }]) {
      state.hour = 100; state.drill = CLOSED; state.slots = [];
      const widget = { ...createWidget('generals', 'main'), time };
      view.render(widget);
      state.table!.onRowClick!(state.table!.rows[0]);
      view.render(widget);
      assert.equal(state.timeline!.labelHour, 112);
      state.host!.onFollow();
      view.render(widget);
      assert.equal(state.timeline!.labelHour, 130);
      assert.equal(state.timeline!.playhead, 131);
      state.timeline!.onContextMenu!(140, 0, 0);
      state.actions!.find(action => action.label === 'Show this hour')!.run();
      const markup = view.render(widget);
      assert.equal(state.host!.following, false);
      assert.equal(state.host!.day, formatDateTime(140));
      assert.equal(state.timeline!.labelHour, 140);
      assert.equal(state.timeline!.playhead, 141);
      assert.ok(state.timeline!.lanes.find(lane => lane.id === 'skill')!.spans.some(span => span.firstHour === 132 && span.lastHour === 140 && span.value === 4));
      assert.ok(state.timeline!.lanes.find(lane => lane.id === 'traits')!.spans.some(span => span.firstHour === 132));
      assert.equal(state.timeline!.lanes.find(lane => lane.id === 'commands')!.spans[0].lastHour, 140);
      assert.match(markup, /<strong>1<\/strong>won/, 'the real stats use the same selected observation as the timeline');
      state.hour = 110;
      view.render(widget);
      assert.equal(state.timeline!.labelHour, 140, 'a picked date stays held while the clock advances');
      state.host!.onPin!();
      assert.deepEqual(view.pinned.at(-1)!.time, { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 40 } } });
    }
  } finally { await view.close(); }
});

test('Generals masks obsolete client drills before effects even when source IDs and reply objects are reused', async () => {
  const data = leaderData();
  const state: State = { hour: 100, drill: CLOSED, slots: [], cursor: 0, subscriptions: [], data, windows: new Map() };
  const view = await fixture(state, '/widgets/generals/GeneralsWidget.tsx');
  try {
    const widget = { ...createWidget('generals', 'main'), time: { mode: 'pin' as const, windows: { main: { firstOffset: 12, lastOffset: 12 } } } };
    const firstClient = view.runtime.client, nextClient = {};
    view.render(widget);
    const oldOpen = state.table!.onRowClick!, oldRow = state.table!.rows[0];
    oldOpen(oldRow);
    view.render(widget);
    const oldHost = state.host!, oldCareer = state.career!, previous = state.drill;
    assert.equal(oldHost.open, true);
    view.runtime.client = nextClient;
    view.render(widget);
    assert.equal(state.data, data, 'the same history object is deliberately supplied by both clients');
    assert.equal(state.host!.open, false);
    assert.equal(state.host!.onPin, undefined);
    assert.equal(state.career, undefined);
    assert.deepEqual(state.subscriptions, [false]);
    oldHost.onFollow(); oldCareer.onPick(140); oldHost.onPin!(); oldOpen(oldRow);
    assert.equal(state.drill, previous, 'obsolete callbacks cannot change or reopen the old selection');
    assert.equal(view.pinned.length, 0, 'obsolete Pin cannot create a widget using the new client');
    state.table!.onRowClick!(state.table!.rows[0]);
    view.render(widget);
    assert.equal(state.host!.open, true, 'the new client can open its own career normally');
    view.runtime.client = firstClient;
    view.render(widget);
    assert.equal(state.host!.open, false, 'A → B → A cannot revive the first A selection');
    assert.equal(state.career, undefined);
    oldOpen(oldRow);
    view.render(widget);
    assert.equal(state.host!.open, false, 'even a callback retained from the first A has obsolete ownership');
  } finally { await view.close(); }
});

test('casualty pair and loss drills date prefix totals in pinned and inline Follow views', async () => {
  const span = { firstHour: 100, lastHour: 160 }, observed = { firstHour: 100, lastHour: 119 };
  const prefix = { coverage: { ...observed, complete: false }, pending: [{ firstHour: 120, lastHour: 160 }] };
  const base = { branchId: '1', available: true, windows: [span], window: 0, observed: [observed], hasGap: false, warnings: [], ...prefix };
  for (const pairs of [true, false]) {
    const reply = pairs ? { ...base, pairs: [{ identity: '1:10|2:20', firstCountry: 'A', secondCountry: 'B',
      runs: [{ ...observed, readHour: 100, hourly: true, firstTaken: '0', secondTaken: '12', incomplete: false }] }] }
      : { ...base, tag: 'A', countries: [{ ...observed, identity: '1:10', incomplete: false }], series: [], wars: [] };
    const state: State = { hour: 120, drill: CLOSED, slots: [], cursor: 0, subscriptions: [],
      data: { ...reply, window: null }, windows: new Map([[0, reply]]) };
    const view = await fixture(state, pairs ? '/widgets/casualties-between/CasualtyPairsWidget.tsx' : '/widgets/casualties-breakdown/CasualtiesBreakdownWidget.tsx');
    try {
      for (const pinned of [false, true]) {
        state.slots = pairs ? pinned ? [] : [casualtyPairsAt(reply as CasualtyPairsReply, 119).flows.map(flow => flow.id)]
          : pinned ? [] : [{ breakdown: 'details', follow: true, hour: null }];
        state.drill = pinned ? CLOSED : { breakdown: 'pairs', follow: true, hour: null };
        const widget: WidgetSpec = { ...createWidget(pairs ? 'casualties-between' : 'casualties-breakdown', 'main'),
          countries: pairs ? { kind: 'all' as const } : { kind: 'tags' as const, tags: ['A'] },
          time: { mode: 'follow' as const }, options: pinned ? { show: pairs ? 'pairs' : 'details' } : {} };
        const html = view.render(widget);
        assert.ok(html.includes(`${pairs ? 'Casualties' : 'Losses'} through ${formatDateTime(119)}; the rest is still being prepared.`), html);
        assert.ok(state.table, 'the prefix totals remain available in the drill');
        if (pairs) assert.equal((state.table.rows[0] as { value: bigint }).value, 12n);
        state.windows = new Map([[0, { ...reply, coverage: { ...span, complete: true }, pending: [] }]]);
        assert.doesNotMatch(view.render(widget), /the rest is still being prepared/);
        state.windows = new Map([[0, reply]]);
      }
    } finally { await view.close(); }
  }
});

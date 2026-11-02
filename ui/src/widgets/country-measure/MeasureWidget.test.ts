import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import type { ComponentType, ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../../charts/frame.ts';
import type { LegendItem } from '../../charts/Legend';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { CountryGroupReply, CountryStabilityReply, MeasureReply, PoliticsLedgerReply } from '../../generated/dashboard';

for (const kind of ['political-power', 'command-power']) test(`${kind} uses mounted dated flag status in its actual legend and tooltip`, async () => {
  type Slot = { value?: unknown; deps?: readonly unknown[] };
  const tags = Array.from({ length: 5000 }, (_, index) => `synthetic actor ${index}`);
  const data: MeasureReply = { branchId: 'branch', measure: kind === 'political-power' ? 'politicalPower' : 'commandPower', decimals: 5, warnings: [], series: tags.map(tag => ({ tag, drops: [], runs: [
    { firstHour: 0, lastHour: 20, valueRaw: '100000', startsSegment: true },
    { firstHour: 21, lastHour: 49, valueRaw: '200000', startsSegment: true },
    { firstHour: 51, lastHour: 100, valueRaw: '300000', startsSegment: false },
  ] })) };
  const state = { cursor: 0, slots: [] as Slot[], effects: [] as (() => unknown)[],
    legend: undefined as { items: LegendItem[]; windowed: boolean; onVisibleItems: (items: readonly LegendItem[]) => void } | undefined,
    chart: undefined as TimeChartProps | undefined, wanted: [] as { tag: string; hour: number }[],
    flagTags: [] as readonly string[], flagHour: 0, exact: false,
    segmentAt: undefined as ((tag: string, hour: number) => number | null) | undefined,
    flag: undefined as string | undefined, flagPending: true, flagReason: undefined as string | undefined,
    failedImage: undefined as string | undefined, clock: 100,
    groups: undefined as CountryGroupReply | undefined, ledger: undefined as PoliticsLedgerReply[] | undefined,
    maxima: undefined as CountryStabilityReply | undefined, maximaTagReads: 0,
    drillHost: undefined as { open: boolean; title: string; canFollow: boolean; onFollow: () => void; onPin?: () => void } | undefined,
    drill: undefined as { hour: number | null; visibleThrough: number; playback: { playhead: number | null } } | undefined,
    added: [] as WidgetSpec[], portals: [] as unknown[],
  };
  const host = globalThis as typeof globalThis & { __powerFlags?: typeof state;
    __powerFlagData?: MeasureReply; __powerHook?: (kind: string, ...args: unknown[]) => unknown;
    __powerFlagHook?: () => [string | undefined, (url: string) => void]; __powerPortal?: (node: unknown) => null };
  host.__powerFlags = state; host.__powerFlagData = data;
  host.__powerFlagHook = () => [state.failedImage, url => { state.failedImage = url; }];
  host.__powerPortal = node => { state.portals.push(node); return null; };
  host.__powerHook = (name, ...args) => {
    const slot = state.slots[state.cursor++] ??= {};
    if (name === 'useMemo' || name === 'useCallback' || name === 'useEffect') {
      const [factory, deps] = args as [() => unknown, readonly unknown[]];
      if (!slot.deps || slot.deps.length !== deps.length || !slot.deps.every((value, index) => Object.is(value, deps[index]))) {
        slot.deps = [...deps];
        if (name === 'useMemo') slot.value = factory();
        else if (name === 'useCallback') slot.value = factory;
        else state.effects.push(factory);
      }
      return slot.value;
    }
    if (!('value' in slot)) slot.value = name === 'useReducer' ? args[1] : name === 'useRef' ? { current: args[0] }
      : typeof args[0] === 'function' ? (args[0] as () => unknown)() : args[0];
    if (name === 'useRef') return slot.value;
    return [slot.value, (next: unknown) => { slot.value = name === 'useReducer'
      ? (args[0] as (before: unknown, action: unknown) => unknown)(slot.value, next)
      : typeof next === 'function' ? (next as (before: unknown) => unknown)(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request, active, validate) => {
      if (!request || !active) return {retry() {}};
      const data = request.product === 'country-measure' ? globalThis.__powerFlagData
        : request.product === 'country-stability' ? globalThis.__powerFlags.maxima : undefined;
      if (request.product !== 'country-measure' && request.product !== 'country-stability') throw Error('Unexpected measure fixture product');
      return {data: data === undefined ? undefined : validate(data), retry() {}};
    };
      export const useProducts = (_client, requests, active, validate) => {
        if (!active || !requests.length) return {retry() {}};
        if (requests.some(request => request.product !== 'politics-ledger')) throw Error('Unexpected measure fixture comparison product');
        const data = globalThis.__powerFlags.ledger;
        return {data: data?.map(validate), retry() {}};
      }; export const loadProduct = () => Promise.resolve({});
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`,
    '/dashboard/useCountryGroups.ts': `import {validateCountryGroupReply} from './groupsReply.ts';
      export const useCountryGroups = (_runtime, _track, active) => ({data: active && globalThis.__powerFlags.groups
        ? validateCountryGroupReply(globalThis.__powerFlags.groups) : undefined, retry() {}});`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__powerFlags.clock;',
    '/dashboard/useCountryLineStyles.ts': `export const useCountryLineStyles = () => ({known: new Map(globalThis.__powerFlagData.series.map(row => [row.tag,
      {tag: row.tag, name: row.tag, color: [20, 50, 80]}])), data: {warnings: []}});`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = (_client, _track, wanted) =>
      new Map(wanted.map(row => [nameKey(row.hour, row.tag), { kind: 'value', value: row.tag }]));`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      globalThis.__powerFlags.wanted.push(...wanted); return {labelAt: () => tag => ({name: tag, flag: globalThis.__powerFlags.flag,
        flagPending: globalThis.__powerFlags.flagPending, flagReason: globalThis.__powerFlags.flagReason, known: true}), revision: 0}; };`,
    '/dashboard/countryLooks.ts': `export const useCountryLooks = (_client, _track, _tags, hour, _active, segmentAt, _hours, exact, flagTags) => {
      Object.assign(globalThis.__powerFlags, {flagHour: hour, segmentAt, exact, flagTags}); return {
        flag: () => globalThis.__powerFlags.flag, flagPending: () => globalThis.__powerFlags.flagPending,
        flagReason: () => globalThis.__powerFlags.flagReason}; };`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({categories: ['#123456'], muted: '#888888', surface: '#000000', countryColours: 'game'});
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({open() {}, menu: null});',
    '/charts/Legend.tsx': 'export default props => { globalThis.__powerFlags.legend = props; return null; };',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__powerFlags.chart = props; return null; };',
    '/charts/drill/DrillHost.tsx': 'export default props => { globalThis.__powerFlags.drillHost = props; return props.open ? props.children : props.main; };',
    '/widgets/country-measure/PowerDrill.tsx': 'export default props => { globalThis.__powerFlags.drill = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'power-flag-adapter', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/components/CountryFlag.tsx')) return source.replace('useState<string>()', 'globalThis.__powerFlagHook()');
        if (path.endsWith('/widgets/country-measure/powerPickModel.ts')) return source.replace('tag: row.tag',
          'get tag() { globalThis.__powerFlags.maximaTagReads++; return row.tag; }');
        if (path.endsWith('/widgets/country-measure/MeasureWidget.tsx')) return source.replace(/\b(useMemo|useCallback|useState|useReducer|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, name) => `globalThis.__powerHook(${JSON.stringify(name)}, `).replace(/\bcreatePortal\(/g, 'globalThis.__powerPortal(');
      },
    }] });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/definitions.ts');
    const { default: registered } = await vite.ssrLoadModule(`/ui/src/widgets/${kind}/index.ts`);
    const Widget = registered.render as ComponentType<WidgetProps>;
    const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [], branches: [] } },
      branch: { id: 'branch', firstHour: 0, lastHour: 100, gaps: [] } };
    const props = { widget: { id: kind, kind, configVersion: 1, trackIds: ['track'], countries: { kind: 'tags', tags },
      time: { mode: 'follow' }, options: { endLabels: false, ceiling: false } }, runtime: { client: {}, tracks: [track],
      clock: { getHour: () => state.clock, getSnapshot: () => ({playing: false, hoursPerSecond: 24}) },
      addWidgetBelow: (_id: string, widget: WidgetSpec) => state.added.push(widget) }, headerSlot: {}, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = (flush = true) => {
      state.cursor = 0; state.effects = []; state.chart = undefined; state.wanted = [];
      state.portals = [];
      const html = renderToStaticMarkup(createElement(Widget, props));
      if (flush) for (const effect of state.effects) effect();
      return html;
    };
    const chart = () => { const chart = state.chart; assert.ok(chart); return chart; };
    const flag = () => state.legend!.items[0].flag as ReactElement;
    const image = () => (flag().type as (props: unknown) => ReactElement)(flag().props);
    render();
    assert.equal(state.legend!.windowed, true);
    assert.equal(state.legend!.items.length, 5000, 'full inventory is retained without mounting every row');
    assert.deepEqual(state.flagTags, []);
    assert.deepEqual(state.wanted, [], 'end and legend names do not eagerly read 5000 flags');
    state.legend!.onVisibleItems([state.legend!.items[0], state.legend!.items[1]]); render();
    assert.deepEqual(state.flagTags, tags.slice(0, 2)); assert.equal(state.flagHour, 100); assert.equal(state.exact, true);
    assert.equal(state.segmentAt!(tags[0], 20), 0); assert.equal(state.segmentAt!(tags[0], 21), 21);
    assert.equal(state.segmentAt!(tags[0], 50), null); assert.equal(state.segmentAt!(tags[0], 51), 51);
    assert.equal(state.segmentAt!('unrecorded actor', 100), null);
    assert.match(renderToStaticMarkup(image()), /Flag loading/);
    state.flagPending = false; state.flagReason = 'No readable flag was returned for this country at this hour.'; render();
    assert.match(renderToStaticMarkup(image()), /Flag unavailable/);
    assert.doesNotMatch(renderToStaticMarkup(image()), /has not loaded|Flag loading/);
    state.flagReason = 'Synthetic recorded-stack refusal'; render();
    assert.match(renderToStaticMarkup(image()), /Flag unavailable/);
    state.flagReason = 'The flag request failed.'; render();
    assert.match(renderToStaticMarkup(image()), /Flag unavailable/);
    state.flag = 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png'; state.flagReason = undefined; render();
    const loaded = image() as ReactElement<{ onError: () => void }>;
    assert.equal(loaded.type, 'img'); loaded.props.onError();
    assert.match(renderToStaticMarkup(image()), /Flag unavailable/);
    state.flag = undefined; state.flagPending = true; state.clock = 21; render();
    assert.equal(state.flagHour, 21); assert.match(renderToStaticMarkup(image()), /Flag loading/);
    state.clock = 100; render();
    chart().tooltip!(21, undefined!, []);
    await Promise.resolve(); render();
    assert.deepEqual(state.wanted, tags.map(tag => ({tag, hour: 21})), 'hover artwork uses the witnessed hour apart from the shown clock');
    const tip = chart().tooltip!(21, undefined!, [])!;
    assert.deepEqual(tip.rows?.[0].countryFlag, {name: tags[0], flag: undefined, flagPending: true, flagReason: undefined, known: true, nameFailed: false});
    state.legend!.onVisibleItems([]); render(); assert.deepEqual(state.flagTags, []);
    props.active = false; state.legend!.onVisibleItems([state.legend!.items[0]]); render(); assert.deepEqual(state.flagTags, []);

    if (kind === 'command-power') {
      state.maximaTagReads = 0;
      state.maxima = { branchId: 'branch', decimals: 5, warnings: [], series: tags.map(tag => ({
        tag, recorded: true, breakdown: null, stability: [], warSupport: [], carried: [],
        maxCommandPower: [{ firstHour: 0, lastHour: 100, valueRaw: '900000', startsSegment: true }],
      })) };
      props.active = true; props.widget.options = { ...props.widget.options, ceiling: true };
      render();
      assert.equal(chart().layers.length, 10001, 'every synthetic country keeps its balance and maximum');
      assert.ok(state.maximaTagReads >= tags.length, 'the counter observes the copied maximum model');
      assert.ok(state.maximaTagReads < 100000, `maximum lookups must not scan the country inventory for every line (${state.maximaTagReads} reads)`);
    }

    // The same serialized request can be served by a different client. Its main
    // balance may arrive before membership, daily details and the CP maximum.
    const actor = tags[0], clientA = props.runtime.client, clientB = {} as typeof clientA;
    const groups: CountryGroupReply = { branchId: 'branch', humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
      countryIdentities: [{ tag: actor, identities: [{ firstHour: 0, lastHour: 100, slot: 1, countryId: 1 }] }],
      unobserved: { humans: [], factions: [], wars: [] }, factions: [{ id: 'native group', firstHour: 0, lastHour: 100, recordedName: 'Synthetic group',
        name: 'Synthetic group', nameReason: null, names: [{ firstHour: 0, lastHour: 100, name: 'Synthetic group', reason: null }],
        members: [{ tag: actor, firstHour: 0, lastHour: 100 }], leaders: [{ tag: actor, firstHour: 0, lastHour: 100 }], incomplete: [] }] };
    const ledger = (hour: number): PoliticsLedgerReply[] => [{ branchId: 'branch', tag: actor, decimals: 5, warnings: [],
      daily: [{ country: 1, instance: 1, epoch: 'synthetic epoch', dayHour: 96, readHour: hour, ppReadHour: hour, cpReadHour: hour,
        ppGrossRaw: '300001', ppCostRaw: '0', ppIncomeRaw: '300001', ppRealizedRaw: '300001', ppSequence: '1',
        cpIncomeRaw: '300001', cpRealizedRaw: '300001', cpSequence: '1' }],
      spending: [], spendItems: [], spendCategories: [], commandHolds: [], ideaTooltips: null,
      spendingCoverage: [{ readHour: 96, sinceHour: 96, throughHour: 100, political: true, command: true, reason: null }] }];
    const maxima = (raw: string): CountryStabilityReply => ({ branchId: 'branch', decimals: 5, warnings: [], series: [{ tag: actor,
      recorded: true, breakdown: null, stability: [], warSupport: [], carried: [],
      maxCommandPower: [{ firstHour: 0, lastHour: 100, valueRaw: raw, startsSegment: true }] }] });
    const balance = (raw: string): MeasureReply => ({ ...data, series: [{ tag: actor, drops: [],
      runs: [{ firstHour: 0, lastHour: 100, valueRaw: raw, startsSegment: true }] }] });
    const rows = () => chart().tooltip!(100, undefined!, [])!.rows!;
    // Daily figures keep the game's two decimals; balances and maxima are whole.
    const daily = kind === 'political-power' ? 'Daily change' : 'Daily gain';
    props.active = true; props.widget.options = { ...props.widget.options, ceiling: true };
    props.widget.countries = { kind: 'group', group: { type: 'faction', id: 'native group' }, membership: 'hour', aggregate: false };
    host.__powerFlagData = balance('700000'); state.groups = groups; state.ledger = ledger(97); state.maxima = maxima('1100001'); render();
    assert.equal(state.drillHost!.open, false);
    assert.equal(state.drillHost!.title, 'Daily income and costs', 'a closed breakdown keeps its neutral name even when group facts already identify its actor');
    assert.deepEqual(state.wanted, [{ tag: actor, hour: 21 }], 'a closed breakdown adds no displayed-day country demand beside the existing hover');
    assert.equal(state.portals.length, 0, 'one current balance stays in its chart/legend without a duplicate header pill');
    const ownerA = chart().pickOwner;
    assert.ok(chart().keyHours!.includes(97));
    assert.equal(rows().find(row => row.label === daily)?.value, '+3.00');
    if (kind === 'command-power') assert.equal(rows().find(row => row.label === 'Maximum')?.value, '11');
    state.groups = undefined; state.ledger = undefined; state.maxima = undefined; render(false);
    assert.equal(chart().pickOwner, ownerA, 'same-client content refresh retains membership, daily geometry and maximum');
    assert.ok(chart().keyHours!.includes(97));
    assert.equal(rows().some(row => row.label === daily), false, 'pending labels do not borrow the old ledger presentation');
    if (kind === 'command-power') assert.equal(rows().find(row => row.label === 'Maximum')?.value, '11');

    props.runtime.client = clientB; host.__powerFlagData = balance('800000'); render(false);
    assert.equal(state.chart, undefined, "new main data cannot borrow another client's still-pending membership before effects");
    state.groups = groups; render(false);
    assert.ok(chart()); assert.notEqual(chart().pickOwner, ownerA);
    assert.deepEqual(chart().keyHours, [0], "new main data cannot borrow the previous client's daily supplying reads");
    assert.equal(rows().some(row => row.label === daily || row.label === 'Maximum'), false);
    assert.equal(chart().layers.length, 2, "the previous client's ceiling is not drawn");
    state.ledger = ledger(99); state.maxima = maxima('2200002'); render();
    assert.ok(chart().keyHours!.includes(99));
    if (kind === 'command-power') assert.equal(rows().find(row => row.label === 'Maximum')?.value, '22');
    const ownerB = chart().pickOwner;
    state.groups = undefined; state.ledger = undefined; state.maxima = undefined;
    props.runtime.client = clientA; host.__powerFlagData = balance('900000'); render(false);
    assert.equal(state.chart, undefined, "A to B to A does not resurrect either visit's membership");
    state.groups = groups; render(false);
    assert.ok(chart()); assert.notEqual(chart().pickOwner, ownerA); assert.notEqual(chart().pickOwner, ownerB);
    assert.deepEqual(chart().keyHours, [0], "the returned client waits for this visit's auxiliary reads");
    assert.equal(rows().some(row => row.label === daily || row.label === 'Maximum'), false);
    assert.equal(chart().layers.length, 2);
    state.ledger = ledger(97); state.maxima = maxima('1100001'); render();
    const sameFactsOwner = chart().pickOwner;
    props.runtime.client = clientB; render(false);
    assert.notEqual(chart().pickOwner, sameFactsOwner, 'client ownership changes even when every reply object and numeric operand is identical');
    props.runtime.client = clientA; render(false);
    assert.notEqual(chart().pickOwner, sameFactsOwner, 'returning to the same client does not restore an obsolete drawn owner');

    props.widget.countries = { kind: 'tags', tags: [actor] };
    track.axisOffset = 10;
    const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
    for (const time of [{ mode: 'campaign' }, { mode: 'range', windows: { track: { firstOffset: 0, lastOffset: 100 } } },
      { mode: 'pin', windows: { track: { firstOffset: 20, lastOffset: 20 } } }] as WidgetSpec['time'][]) {
      props.runtime.client = {} as typeof clientA;
      props.widget.time = time;
      state.clock = 30;
      render(); render();
      chart().onPick!(20, { frame, until: null, point: { x: frame.toX(20), y: frame.toY(9) } });
      render();
      assert.equal(state.drill!.hour, 20, `${time.mode} opens at the drawn date`);
      assert.equal(state.drillHost!.title, `${actor}: Daily income and costs`, 'the open breakdown names its dated actor');
      assert.equal(state.drillHost!.canFollow, true, `${time.mode} can explicitly follow playback`);
      state.drillHost!.onFollow(); render();
      assert.equal(state.drill!.hour, 40, 'Follow uses the source-aligned playback hour');
      assert.equal(state.drill!.playback.playhead, 40);
      assert.equal(state.drill!.visibleThrough, 40, 'later reads remain unavailable while following');
      state.clock = 45; render();
      assert.equal(state.drill!.hour, 55);
      state.drillHost!.onPin!();
      assert.equal(state.added.at(-1)!.time.mode, 'follow', 'pinning the followed drill retains Follow');
      assert.deepEqual(JSON.parse(state.added.at(-1)!.options!.powerCountry as string),
        { campaign: 'campaign', branch: 'branch', tag: actor, originOffset: 20, slot: 1, countryId: 1 });
      props.runtime.client = {} as typeof clientA;
      render(false);
      assert.equal(state.drillHost!.open, false, 'a different client masks the previous native drill before effects');
    }
  } finally { await vite.close(); delete host.__powerFlags; delete host.__powerFlagData; delete host.__powerHook; delete host.__powerFlagHook; delete host.__powerPortal; }
});

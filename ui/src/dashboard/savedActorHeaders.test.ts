import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement, isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps, WidgetRuntime } from './definitions';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';
import type { CountryRaceViewProps } from '../charts/race/CountryRaceView';
import type { TimeChartProps } from '../charts/TimeChart';
import type { ArmyRun, ArmySizeReply, CountryNukesReply, MilitaryAirReply, SupplyHealthReply } from '../generated/dashboard';

const span = (firstHour: number, lastHour: number) => ({ firstHour, lastHour });
const identity = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ ...span(firstHour, lastHour), slot, countryId });
const identities = [
  { tag: 'X', identities: [identity(0, 9, 1, 10), identity(10, 14, 2, 20), identity(15, 20, 3, 30)] },
  { tag: 'Y', identities: [identity(15, 20, 2, 20)] },
];
const army: ArmySizeReply = { branchId: '1', available: true, measures: ['divisions'], unknownMeasures: [], types: [], warnings: [],
  windows: [], window: 0, interval: span(0, 20), series: identities.map(history => ({ ...history, membership: null,
    segments: [span(0, 20)], runs: history.identities.map<ArmyRun>(run => [run.firstHour, run.slot === 3 ? 99 : run.slot * 11, run.lastHour]) })) };
const manifest: ArmySizeReply = { ...army, windows: [span(0, 20)], window: null, interval: null, series: [] };
const supply: SupplyHealthReply = { branchId: '1', available: true, reason: null, missingInputs: [], warnings: [],
  series: identities.map(history => ({ ...history, runs: history.identities.map(run => ({ ...span(run.firstHour, run.lastHour),
    startsSegment: true, noSupply: run.slot === 3 ? 99 : run.slot * 11, strained: 0, inSupply: 0 })) })) };
const nuclear: CountryNukesReply = { branchId: '1', readHours: [0, 10, 15], emptyReadHours: [], types: [], warnings: [],
  series: identities.map(history => ({ tag: history.tag, membership: null, segments: history.identities.map(run => span(run.firstHour, run.lastHour)),
    runs: history.identities.map(run => ({ ...span(run.firstHour, run.lastHour), startsSegment: true, total: run.slot === 3 ? 99 : run.slot * 11, counts: [] })) })) };
const nuclearOrigin: CountryNukesReply = { ...nuclear, series: nuclear.series.map(series => ({ ...series,
  membership: [series.tag === 'X' ? span(10, 14) : span(15, 20)],
  segments: [series.tag === 'X' ? span(10, 14) : span(15, 20)],
  runs: [{ ...(series.tag === 'X' ? span(10, 14) : span(15, 20)), startsSegment: true, total: 22, counts: [] }] })) };
const ledger: MilitaryAirReply = { family: 'airLosses', firstHour: 0, lastHour: 20, available: true, observed: [span(0, 20)], incomplete: [],
  countries: [{ tag: 'X', identity: 'A', present: [span(0, 9)], membership: null },
    { tag: 'X', identity: 'B', present: [span(10, 14)], membership: null },
    { tag: 'X', identity: 'C', present: [span(15, 20)], membership: null },
    { tag: 'Y', identity: 'B', present: [span(15, 20)], membership: null }],
  parts: [], regions: [], buildings: [], warnings: [] };

test('registered saved native-actor bodies supply country headers at their displayed date', async () => {
  const state = { hour: 12, pending: false, error: undefined as Error | undefined, content: 0,
    army, supply, nuclear, nuclearOrigin, ledger, armyWindows: new Map([[0, army]]), windowErrors: new Map<number, string>(),
    nameKnown: true as boolean | undefined, nameFailed: false, headerName: undefined as string | undefined,
    flagPending: false, flagReason: undefined as string | undefined, flag: 'data:flag-B' as string | undefined,
    hooks: new Map<string, unknown[]>(), owner: '', cursor: 0, changed: false, effects: [] as (() => void)[],
    elements: new Map<string, ReactNode>(), widgetProps: undefined as WidgetProps | undefined,
    slot: { id: 'country' } as unknown as HTMLElement, summarySlot: { id: 'summary' } as unknown as HTMLElement,
    portals: [] as { chip: ReactElement<{ label?: { name: string; flag?: string } | null; hour: number | null; reason: string | null }>; slot: HTMLElement }[],
    reads: [] as { client: unknown; sourceId: string | undefined; revision: string | undefined; wanted: { tag: string; hour: number }[] }[],
    defaults: [] as number[], products: [] as { client: unknown; request: { sourceId: string; revision: string; product: string } }[],
    race: undefined as CountryRaceViewProps | undefined, charts: [] as TimeChartProps[],
    butterfly: undefined as { items: { row: unknown }[]; onSelect: (item: unknown, side: string) => void } | undefined,
    host: undefined as { onPin?: () => void } | undefined, added: [] as WidgetSpec[],
    picker: undefined as { onChange: (tag: string | null) => void } | undefined };
  const globals = globalThis as typeof globalThis & { __savedActorHeaders?: typeof state };
  globals.__savedActorHeaders = state;
  // Preserve each real body's hook cells and effects; replace only transport,
  // canvas/popover leaves and DOM portal placement.
  const hooks = `export const withHooks = (owner, render, props) => {
      const state = globalThis.__savedActorHeaders, previous = [state.owner, state.cursor]; state.owner = owner; state.cursor = 0;
      if (owner.endsWith('Widget')) state.widgetProps = props;
      try { const node = render(); state.elements.set(owner, node); return node; } finally { [state.owner, state.cursor] = previous; } };
    const cell = make => { const state = globalThis.__savedActorHeaders; let cells = state.hooks.get(state.owner);
      if (!cells) { cells = []; state.hooks.set(state.owner, cells); } const index = state.cursor++; return cells[index] ??= make(); };
    export const useEffect = (make, deps) => { const saved = cell(() => ({ deps: undefined, cleanup: undefined }));
      if (!saved.deps || saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) {
        saved.deps = deps; globalThis.__savedActorHeaders.effects.push(() => { saved.cleanup?.(); saved.cleanup = make(); }); } };
    export const useLayoutEffect = useEffect;
    export const useRef = initial => cell(() => ({ current: initial }));
    export const useMemo = (make, deps) => { const saved = cell(() => ({ deps, value: make() }));
      if (saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) { saved.deps = deps; saved.value = make(); } return saved.value; };
    export const useCallback = (callback, deps) => useMemo(() => callback, deps);
    export const useState = initial => { const saved = cell(() => ({ value: typeof initial === 'function' ? initial() : initial, set: undefined }));
      saved.set ??= next => { const value = typeof next === 'function' ? next(saved.value) : next;
        if (!Object.is(value, saved.value)) globalThis.__savedActorHeaders.changed = true; saved.value = value; };
      return [saved.value, saved.set]; };
    export const useReducer = (reduce, initial) => { const saved = cell(() => ({ value: initial, dispatch: undefined, reduce })); saved.reduce = reduce;
      saved.dispatch ??= action => { const value = saved.reduce(saved.value, action);
        if (!Object.is(value, saved.value)) globalThis.__savedActorHeaders.changed = true; saved.value = value; };
      return [saved.value, saved.dispatch]; };`;
  const leaves: Record<string, string> = {
    '/dashboard/products.ts': `const manifest = ${JSON.stringify(manifest)};
      export function useProduct(client, request, active, validate) {
        const state = globalThis.__savedActorHeaders; if (!request || !active) return { retry() {} };
        state.products.push({ client, request }); if (state.pending || state.error) return { error: state.error, retry() {} };
        const reply = request.product === 'army-size' ? manifest : request.product === 'supply-health' ? state.supply
          : request.product === 'country-nukes' ? request.params.originHour == null ? state.nuclear : state.nuclearOrigin
          : request.product === 'military-air-ledger' ? { ...state.ledger, family: request.params.family } : null;
        try { return { data: reply ? validate(reply) : undefined, retry() {} }; } catch (error) { return { error: error.message, retry() {} }; }
      } export const loadProduct = (client, request, _signal, validate, _source, _restart, publish) => {
        if (!publish) return Promise.resolve({ flags: [] });
        const { data, error } = useProduct(client, request, true, validate); if (error) throw Error(error);
        if (data !== undefined) publish(data); return new Promise(() => {}); };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    // View reads answer in the same render through the transport above.
    '/dashboard/viewRead.ts': `export const useViewRead = (lane, view, _viewKey, active, read) => { let data, error;
      if (active && lane != null && view != null) try { void read(view, new AbortController().signal, value => { data = value; }); } catch (failure) { error = failure.message; }
      return { data, error, retry() {} }; };`,
    '/dashboard/productWindows.ts': `export function useProductWindows(client, _identity, request, wanted, active, validate) {
      const state = globalThis.__savedActorHeaders; if (active) for (const window of wanted) state.products.push({ client, request: request(window) });
      if (!active || state.pending) return { data: new Map(), errors: new Map(), retry() {} };
      try { for (const value of state.armyWindows.values()) validate(value);
        return { data: state.armyWindows, errors: state.error ? new Map([[0, state.error]]) : state.windowErrors, retry() {} };
      } catch (error) { return { data: new Map(), errors: new Map([[0, error.message]]), retry() {} }; } }`,
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ retry() {} });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (client, track, wanted) => {
      const state = globalThis.__savedActorHeaders; state.reads.push({ client, sourceId: track?.source.id, revision: track?.source.revision, wanted });
      return { labelAt: hour => tag => ({ name: state.headerName ?? (tag === 'Y' ? 'Renamed Beta' : 'Beta'),
        flag: state.flag ? state.flag + '@' + hour : undefined, known: state.nameKnown, nameFailed: state.nameFailed, flagPending: state.flagPending, flagReason: state.flagReason }), revision: 0 }; };`,
    '/dashboard/campaignSettings.ts': 'export const useCampaignSettings = () => ({ campaigns: {} });',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => globalThis.__savedActorHeaders.content;',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__savedActorHeaders.hour; export const usePlaybackMotion = () => 24;',
    '/dashboard/WidgetSettingsMenu.tsx': 'export default () => null;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => tag, flag: () => undefined,
      flagReason: () => undefined, flagPending: () => false, color: () => '#123' });`,
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }) });',
    '/charts/race/countryColours.ts': "export const useCountryColours = () => () => '#123';",
    '/charts/useChartTokens.ts': "export const useViewportHeight = () => 1000; export const useChartTokens = () => ({ muted: '#888', categories: ['#123'], error: '#a00', warning: '#aa0', positive: '#0a0' });",
    '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__savedActorHeaders.race = props; return createElement('div', null, props.state, props.beforeChart); };`,
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__savedActorHeaders.charts.push(props); return null; };',
    '/charts/StackedShareChart.tsx': 'export default props => { globalThis.__savedActorHeaders.charts.push(props); return null; };',
    '/charts/drill/DrillParts.tsx': 'export default () => null;',
    '/charts/sunburst/Sunburst.tsx': 'export default () => null;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__savedActorHeaders.picker = props; return null; };',
    '/dashboard/BombingDrill.tsx': 'export default () => null;',
    '/charts/butterfly/index.ts': 'export const ButterflyChart = props => { globalThis.__savedActorHeaders.butterfly = props; return null; };',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__savedActorHeaders.host = props; return createElement('div', null, props.main, props.open ? props.children : null); };`,
  };
  const mounted = ['ArmySizeWidget', 'ArmyBreakdown', 'SupplyHealthWidget', 'SupplyBreakdown', 'NuclearArsenalWidget',
    'MilitaryAirView', 'AirKillsLossesWidget', 'BombingDamageWidget', 'AirLossDrill', 'BombingDamageBreakdownWidget', 'BombingBreakdown'];
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'saved-actor-header-leaves', enforce: 'pre',
      resolveId: id => id === 'react-grid-layout' ? '\0saved-actor-grid' : id === 'saved-actor-hooks' ? '\0saved-actor-hooks'
        : id === 'saved-actor-portals' ? '\0saved-actor-portals' : undefined,
      load: id => id === '\0saved-actor-hooks' ? hooks : id === '\0saved-actor-grid'
        ? `export const useContainerWidth = () => ({ width: 1200, containerRef: null, mounted: true }); export const Responsive = ({ children }) => children;`
        : id === '\0saved-actor-portals' ? `import { createElement } from 'react'; export const createPortal = (chip, slot) => {
          globalThis.__savedActorHeaders.portals.push({ chip, slot }); return createElement('span', { 'data-country-target': slot.id }, chip); };` : undefined,
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), leaf = Object.entries(leaves).find(([suffix]) => path.endsWith(suffix));
        if (leaf) return leaf[1];
        if (path.endsWith('/dashboard/DashboardGrid.tsx')) return source
          .replace('const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);',
            'const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(globalThis.__savedActorHeaders.summarySlot);')
          .replace('const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);',
            'const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(globalThis.__savedActorHeaders.slot);');
        const name = mounted.find(name => path.endsWith('/' + name + '.tsx'));
        if (name) return source.replace("from 'react'", "from 'saved-actor-hooks'")
          .replace("import { createPortal } from 'react-dom';", "import { createPortal } from 'saved-actor-portals';")
          .replace('export default function ', 'function ')
          + '\nimport { withHooks } from "saved-actor-hooks";\nexport default props => withHooks("' + name + '", () => ' + name + '(props), props);';
        if (path.endsWith('/components/CountryFlag.tsx')) return source.slice(0, source.indexOf('export function SourceFlag')) +
          `export function SourceFlag({ source, tag }) { const country = source.metadata.countries.find(country => country.tag === tag);
            globalThis.__savedActorHeaders.defaults.push(country.observedHour); return <img src="data:flag-A" alt="" />; }`;
      },
    }] });
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts') as typeof import('./definitions.ts');
    const model = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('./model.ts');
    const { parseTemplate } = await vite.ssrLoadModule('/ui/src/dashboard/storage.ts') as typeof import('./storage.ts');
    const Grid = (await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx')).default;
    const kinds = ['army-size', 'supply-health', 'nuclear-arsenal', 'air-kills-losses', 'bombing-damage-breakdown'];
    for (const kind of [...kinds, 'bombing-damage']) registry.registerWidgets([{
      ...(await vite.ssrLoadModule('/ui/src/widgets/' + kind + '/definition.ts')).definition,
      ...(await vite.ssrLoadModule('/ui/src/widgets/' + kind + '/index.ts')).default }]);
    const template: DashboardTemplate = { version: 1, name: 'Synthetic', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: 'X' }],
      tracks: [{ id: 'track', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: null }], widgets: [], layouts: {} };
    const track = { ...template.tracks[0], axisOffset: 0, selectedTag: 'X', branch: { id: '1', firstHour: 0, lastHour: 20 },
      source: { id: 'source-A', revision: '1', metadata: { campaignKey: 'campaign', playerCountryId: 10,
        branches: [{ id: '1', firstHour: 0, lastHour: 20 }],
        countries: [{ id: 10, tag: 'X', name: 'Alpha', observedBranchId: '1', observedHour: 0 }] } } } as unknown as BoundTrack;
    const runtime = { tracks: [track], campaignTracks: template.tracks, defaultRoleId: 'primary', client: {}, maps: { schedule() {} },
      clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) },
      addWidgetBelow: (_id: string, widget: WidgetSpec) => state.added.push(widget) } as unknown as WidgetRuntime;
    const armyPin = await vite.ssrLoadModule('/ui/src/widgets/army-size/armyDrill.ts') as typeof import('../widgets/army-size/armyDrill.ts');
    const parts = await vite.ssrLoadModule('/ui/src/charts/race/partsRaceDrill.ts') as typeof import('../charts/race/partsRaceDrill.ts');
    const nukes = await vite.ssrLoadModule('/ui/src/widgets/nuclear-arsenal/nuclearArsenalData.ts') as typeof import('../widgets/nuclear-arsenal/nuclearArsenalData.ts');
    const air = await vite.ssrLoadModule('/ui/src/dashboard/airLossDrill.ts') as typeof import('./airLossDrill.ts');
    const view = (await vite.ssrLoadModule('/ui/src/dashboard/MilitaryAirView.tsx')).default;
    const { default: FollowControl } = await vite.ssrLoadModule('/ui/src/charts/drill/FollowControl.tsx') as typeof import('../charts/drill/FollowControl');
    function findButton(node: ReactNode, label: string): ReactElement<{ onClick: () => void }> | null {
      if (Array.isArray(node)) return node.map(child => findButton(child, label)).find(Boolean) ?? null;
      if (!isValidElement<{ children?: ReactNode; onClick?: () => void; 'aria-label'?: string }>(node)) return null;
      if (node.type === FollowControl) return findButton(FollowControl(node.props as unknown as Parameters<typeof FollowControl>[0]), label);
      if (node.type === 'button' && (node.props.children === label || node.props['aria-label'] === label)) return node as ReactElement<{ onClick: () => void }>;
      return findButton(node.props.children, label);
    }
    const unmount = () => { for (const cells of state.hooks.values()) for (const cell of cells as { cleanup?: () => void }[]) cell.cleanup?.();
      state.hooks.clear(); state.elements.clear(); state.effects = []; };
    const settle = (draw: () => string) => { for (let round = 0; round < 16; round++) {
      state.changed = false; state.products = []; state.reads = []; state.defaults = []; state.portals = []; state.charts = [];
      const html = draw(), effects = state.effects; state.effects = []; for (const effect of effects) effect();
      if (!state.changed) return html; } throw new Error('The mounted body did not settle.'); };
    const render = (widget: WidgetSpec) => { template.widgets = [widget]; return settle(() => renderToStaticMarkup(createElement(Grid,
      { template, runtime, editing: false, onLayouts() {}, onApply() {}, onRemove() {} }))); };
    const save = (widget: WidgetSpec) => { template.widgets = [widget]; template.layouts = model.addGridItem({}, widget);
      const restored = parseTemplate(JSON.parse(JSON.stringify(template))); assert.ok(restored); return restored.widgets[0]; };
    const countryPortal = () => state.portals.find(portal => portal.slot === state.slot);
    const expectDate = (widget: WidgetSpec, hour: number, tag: string) => {
      const html = render(widget), portal = countryPortal(); assert.ok(portal, widget.kind);
      assert.equal(portal.chip.props.hour, hour); assert.equal(portal.chip.props.reason, null);
      assert.equal(portal.chip.props.label?.name, tag === 'Y' ? 'Renamed Beta' : 'Beta');
      assert.equal(portal.chip.props.label?.flag, 'data:flag-B@' + hour, 'the rendered flag uses the same displayed body date');
      assert.ok(state.reads.some(read => read.wanted.some(wanted => wanted.tag === tag && wanted.hour === hour)));
      assert.match(html, /data:flag-B/); assert.doesNotMatch(html, />Alpha<|data:flag-A/);
      assert.deepEqual(state.defaults, [], 'the frame never invokes a metadata SourceFlag for a breakdown');
      assert.equal(state.widgetProps!.countrySlot, state.slot, 'actual Grid and WidgetBody hand over the country target');
      assert.equal(state.widgetProps!.headerSlot, state.summarySlot, 'the title target stays independent');
      return html;
    };
    const neutral = (widget: WidgetSpec) => { const html = render(widget);
      assert.doesNotMatch(html, />Alpha<|data:flag-A/); assert.deepEqual(state.defaults, []);
      const portal = countryPortal(); if (portal) {
        assert.equal(portal.chip.props.label, null);
        const chip = renderToStaticMarkup(portal.chip); assert.match(chip, />Unavailable<\/span>/); assert.doesNotMatch(chip, /data:flag-B/);
      } return html; };
    for (const kind of kinds) {
      state.hour = 12; unmount();
      const original = { ...model.createWidget(kind, 'track', 'saved-' + kind), campaign: 'tracks' as const,
        countries: { kind: 'tags' as const, tags: ['X'] },
        time: { mode: 'pin' as const, windows: { track: { firstOffset: 12, lastOffset: 12 } } } };
      let pinned: WidgetSpec;
      if (kind === 'army-size') pinned = armyPin.pinArmyDrill(original, track, 'X', 12, 12, 'divisions', false);
      else if (kind === 'supply-health') pinned = parts.pinPartsRace({ ...original, options: { drillOriginHour: 12 } }, track, 'X', 12, false, 'Supply classification');
      else if (kind === 'nuclear-arsenal') pinned = nukes.pinNuclearDrill(original, track, 'X', 12, 12, false);
      else if (kind === 'air-kills-losses') pinned = air.pinAirLossDrill(original, track, 'X', 'B', 0, 12, false);
      else {
        const bombing = { ...original, kind: 'bombing-damage', title: 'Bombing damage' };
        const props = { widget: bombing, runtime, active: true, onDiagnostics() {}, family: 'bombing', fields: [], title: 'Bombing damage' };
        const mount = () => settle(() => renderToStaticMarkup(view(props))); mount();
        const item = state.butterfly!.items.find(item => (item.row as { identity: string }).identity === 'B')!;
        assert.ok(item); state.butterfly!.onSelect(item, 'left'); mount(); state.host!.onPin!(); pinned = state.added.at(-1)!;
      }
      pinned = save(pinned); unmount();
      const savedTime = JSON.stringify(pinned.time);
      expectDate(pinned, 12, 'X');
      if (kind === 'supply-health') {
        assert.ok(state.portals.some(portal => portal.slot === state.summarySlot), 'Supply retains its existing subtitle portal');
        assert.equal(state.portals.filter(portal => portal.slot === state.slot).length, 1);
      }
      if (kind !== 'bombing-damage-breakdown') {
        const pick = kind === 'army-size' || kind === 'nuclear-arsenal' ? state.race!.onPick! : state.charts.at(-1)!.onPick!;
        (pick as (hour: number) => void)(15); expectDate(pinned, 15, 'Y');
        state.hour = 17;
        const follow = kind === 'army-size' || kind === 'nuclear-arsenal' ? findButton(state.race!.beforeChart, 'Follow playback')
          : [...state.elements.values()].map(node => findButton(node, 'Follow playback')).find(Boolean);
        assert.ok(follow, kind + ' uses its actual existing Follow button'); follow.props.onClick(); expectDate(pinned, 17, 'Y');
        assert.equal(JSON.stringify(pinned.time), savedTime, 'local history and Follow do not write WidgetSpec time');
      } else {
        expectDate({ ...pinned, time: { mode: 'pin', windows: { track: { firstOffset: 15, lastOffset: 15 } } } }, 15, 'Y');
        state.hour = 17;
        expectDate({ ...pinned, time: { mode: 'follow' } }, 17, 'Y');
      }
      state.hour = 12; unmount(); expectDate(pinned, 12, 'X');
      state.nameKnown = undefined; assert.match(render(pinned), />Loading…<\/span>/);
      assert.match(render(pinned), /data:flag-B/, 'an authenticated flag remains independent of a pending name');
      state.nameKnown = false; assert.match(render(pinned), />Unavailable<\/span>/);
      assert.match(render(pinned), /data:flag-B/, 'an authenticated flag remains independent of the unavailable name');
      state.nameKnown = true; state.nameFailed = true; assert.match(render(pinned), />Couldn’t load country name\.<\/span>/);
      assert.match(render(pinned), /data:flag-B/, 'an authenticated flag remains independent of a failed name read');
      state.flag = undefined; state.flagReason = 'Concurrent native flag refusal'; state.nameFailed = false; state.nameKnown = false;
      const refused = render(pinned); assert.match(refused, />Unavailable<\/span>/);
      assert.match(refused, /aria-label="Flag unavailable"/);
      state.nameKnown = true; state.nameFailed = true; state.flagReason = 'Distinct native flag request failed';
      const failed = render(pinned); assert.match(failed, />Couldn’t load country name\.<\/span>/);
      assert.match(failed, /aria-label="Flag unavailable"/);
      state.flagReason = undefined; state.flagPending = true;
      assert.match(render(pinned), /Flag loading/, 'pending artwork remains independent of a failed name');
      state.flag = 'data:flag-B'; state.flagPending = false;
      state.nameFailed = false; state.headerName = ''; assert.doesNotMatch(render(pinned), /Unavailable/);
      assert.equal(countryPortal()!.chip.props.label!.name, '', 'a known empty native name stays empty'); state.headerName = undefined;
      state.flag = undefined; state.flagPending = true; assert.match(render(pinned), /Flag loading/);
      state.flagPending = false; state.flagReason = 'Synthetic content refusal'; assert.doesNotMatch(render(pinned), /Synthetic content refusal/);
      state.flagReason = 'The flag request failed.'; assert.doesNotMatch(render(pinned), /flag request failed/);
      state.flag = 'data:flag-B'; state.flagReason = undefined;
      state.pending = true; neutral(pinned);
      const client = runtime.client, source = track.source;
      runtime.client = {} as never; track.source = { ...source, id: 'source-B', revision: '2' }; state.content++; neutral(pinned);
      state.pending = false; unmount(); expectDate(pinned, 12, 'X');
      assert.ok(state.products.every(read => read.client === runtime.client && read.request.sourceId === 'source-B' && read.request.revision === '2'));
      assert.ok(state.reads.every(read => read.client === runtime.client && read.sourceId === 'source-B' && read.revision === '2'));
      runtime.client = client; track.source = source; unmount();
      state.error = new Error('Synthetic identity refusal'); neutral(pinned); state.error = undefined; unmount();
      unmount(); neutral({ ...pinned, time: { mode: 'pin', windows: {} } });
      const rawOptions = { ...pinned.options };
      delete rawOptions.drillOriginHour; delete rawOptions.holder;
      // Without its origin the breakdown is unpinned: its header follows what the body shows.
      if (kind === 'army-size') {
        unmount(); state.picker = undefined; render({ ...pinned, options: rawOptions });
        assert.ok(state.picker, 'this Army body asks which holder to open');
        assert.match(renderToStaticMarkup(countryPortal()!.chip), />Choose a country</); unmount();
      } else if (kind === 'supply-health') { unmount(); expectDate({ ...pinned, options: rawOptions }, 12, 'X'); unmount(); }
      else neutral({ ...pinned, options: rawOptions });
      const invalid = { ...pinned, options: { ...pinned.options, ...(kind === 'air-kills-losses' || kind === 'bombing-damage-breakdown'
        ? { holder: JSON.stringify(['another-campaign', '1', 'X', 'B']) } : { drillOriginHour: 21 }) } };
      unmount(); neutral(invalid); assert.ok(countryPortal(), 'a loaded malformed witness has an explicit neutral portal');
      unmount();
      for (const malformed of kind === 'air-kills-losses' || kind === 'bombing-damage-breakdown' ? ['{}', true, null] : ['12', 12.5, null]) {
        const malformedOptions = { ...pinned.options, ...(kind === 'air-kills-losses' || kind === 'bombing-damage-breakdown'
          ? { holder: malformed } : { drillOriginHour: malformed }) };
        unmount(); neutral({ ...pinned, options: malformedOptions });
        assert.ok(countryPortal(), 'a loaded malformed witness keeps explicit unavailable country presentation');
      }
      unmount();
      state.army = { ...army, series: army.series.map(series => ({ ...series, identities: [] })) }; state.armyWindows = new Map([[0, state.army]]);
      state.supply = { ...supply, series: supply.series.map(series => ({ ...series, identities: [] })) };
      state.nuclearOrigin = { ...nuclearOrigin, series: nuclearOrigin.series.map(series => ({ ...series, membership: [] })) };
      state.ledger = { ...ledger, countries: ledger.countries.filter(country => country.identity !== 'B') };
      neutral(pinned);
      state.army = army; state.armyWindows = new Map([[0, army]]); state.supply = supply; state.nuclearOrigin = nuclearOrigin; state.ledger = ledger;
      unmount();
      state.hour = 17; unmount();
      const gapIdentities = [identity(15, 16, 2, 20), identity(18, 20, 2, 20)];
      state.army = { ...army, series: army.series.map(series => series.tag === 'Y' ? { ...series, identities: gapIdentities } : series) };
      state.armyWindows = new Map([[0, state.army]]);
      state.supply = { ...supply, series: supply.series.map(series => series.tag === 'Y' ? { ...series, identities: gapIdentities } : series) };
      state.nuclearOrigin = { ...nuclearOrigin, series: nuclearOrigin.series.map(series => series.tag === 'Y'
        ? { ...series, membership: [span(15, 16), span(18, 20)] } : series) };
      state.ledger = { ...ledger, countries: ledger.countries.map(country => country.identity === 'B' && country.tag === 'Y'
        ? { ...country, present: [span(15, 16), span(18, 20)] } : country) };
      neutral({ ...pinned, time: { mode: 'follow' } });
      assert.ok(state.reads.every(read => read.wanted.every(wanted => !(wanted.tag === 'X' && wanted.hour === 17))),
        'a current identity gap never asks for replacement X/C');
      state.army = army; state.armyWindows = new Map([[0, army]]); state.supply = supply; state.nuclearOrigin = nuclearOrigin; state.ledger = ledger;
      state.hour = 12; unmount();
      if (kind === 'army-size' || kind === 'supply-health') {
        state.army = { ...army, series: army.series.map(({ identities: _identities, ...series }) => series) };
        state.armyWindows = new Map([[0, state.army]]);
        state.supply = { ...supply, series: supply.series.map(({ identities: _identities, ...series }) => series) };
        neutral(pinned);
        state.army = army; state.armyWindows = new Map([[0, army]]); state.supply = supply; unmount();
      }
      state.army = { ...army, series: [...army.series, { ...army.series[1], tag: 'CONFLICT', identities: [identity(10, 20, 2, 20)] }] };
      state.armyWindows = new Map([[0, state.army]]);
      state.supply = { ...supply, series: [...supply.series, { ...supply.series[1], tag: 'CONFLICT', identities: [identity(10, 20, 2, 20)] }] };
      state.nuclearOrigin = { ...nuclearOrigin, series: [...nuclearOrigin.series, { ...nuclearOrigin.series[0], tag: 'CONFLICT' }] };
      state.ledger = { ...ledger, countries: [...ledger.countries, { tag: 'CONFLICT', identity: 'B', present: [span(10, 20)], membership: null }] };
      neutral(pinned);
      state.army = army; state.armyWindows = new Map([[0, army]]); state.supply = supply; state.nuclearOrigin = nuclearOrigin;
      state.ledger = { ...ledger, countries: ledger.countries.map(country => country.identity === 'A' ? { ...country, present: [span(0, 14)] } : country) };
      if (kind === 'air-kills-losses' || kind === 'bombing-damage-breakdown') { unmount(); neutral(pinned); }
      state.ledger = ledger; unmount();
      state.slot = { id: 'country-replacement' } as unknown as HTMLElement; expectDate(pinned, 12, 'X');
      assert.equal(countryPortal()!.slot, state.slot, 'a replacement DOM target receives the current body chip');
      unmount();
      const extra = Array.from({ length: 4997 }, (_, index) => ({ tag: 'synthetic-' + index,
        identities: [identity(0, 20, index + 4, index + 40)] }));
      state.army = { ...army, series: [...army.series, ...extra.map(history => ({ ...history, membership: null,
        segments: [span(0, 20)], runs: [[0, 0, 20] as ArmyRun] }))] }; state.armyWindows = new Map([[0, state.army]]);
      state.supply = { ...supply, series: [...supply.series, ...extra.map(history => ({ ...history,
        runs: [{ ...span(0, 20), startsSegment: true, noSupply: 0, strained: 0, inSupply: 0 }] }))] };
      state.nuclear = { ...nuclear, series: [...nuclear.series, ...extra.map(history => ({ tag: history.tag, membership: null,
        segments: [span(0, 20)], runs: [{ ...span(0, 20), startsSegment: true, total: 0, counts: [] }] }))] };
      state.ledger = { ...ledger, countries: [...ledger.countries, ...extra.map(history => ({ tag: history.tag,
        identity: history.tag, present: [span(0, 20)], membership: null }))] };
      expectDate(pinned, 12, 'X');
      assert.ok(state.reads.every(read => read.wanted.length <= 1), '5,000 native actors still plan only the selected header alias');
      state.army = army; state.armyWindows = new Map([[0, army]]); state.supply = supply; state.nuclear = nuclear; state.ledger = ledger;
      const main = { ...original, kind: kind === 'bombing-damage-breakdown' ? 'bombing-damage' : kind, options: {} };
      unmount(); assert.match(render(main), />Alpha</); assert.deepEqual(state.defaults, [0]);
      assert.equal(state.widgetProps!.countrySlot, state.slot, 'fixture retains the prior slot until React ref cleanup');
      assert.doesNotMatch(render(main), /dash-widget-country-slot/, 'MAIN unmounts the country target and renders no body country portal');
      assert.equal(countryPortal(), undefined);
      if (kind !== 'bombing-damage-breakdown') for (const show of [true, 1, null, 'invalid']) {
        unmount(); assert.match(render({ ...main, options: { show } }), />Alpha</, 'illegal show normalizes to MAIN');
        assert.equal(countryPortal(), undefined);
      }
    }
    // A catalogue card asks for a country until one is chosen in its body, then names it at the body's date.
    for (const [kind, key] of [['army-size', 'view=breakdown'], ['supply-health', 'show=breakdown']]) {
      const variant = registry.widgetDefinition(kind as WidgetSpec['kind'])!.variants!.find(item => item.key === key)!;
      const card = { ...model.createWidget(kind as WidgetSpec['kind'], 'track', 'card-' + kind), ...variant.initial, campaign: 'tracks' as const,
        countries: { kind: 'tags' as const, tags: ['X', 'Y'] }, time: { mode: 'follow' as const } };
      state.hour = 17; unmount(); state.picker = undefined;
      render(card);
      const chip = renderToStaticMarkup(countryPortal()!.chip);
      assert.match(chip, />Choose a country</, kind + ' card asks for a country before one is chosen');
      assert.doesNotMatch(chip, /Unavailable/);
      const picker = state.picker as { onChange: (tag: string | null) => void } | undefined;
      assert.ok(picker, kind + ' card offers its countries in the body');
      picker.onChange('Y'); expectDate(card, 17, 'Y');
      assert.equal(card.options?.drillOriginHour, undefined, 'choosing in the body writes no pin');
    }
  } finally { await vite.close(); delete globals.__savedActorHeaders; }
});

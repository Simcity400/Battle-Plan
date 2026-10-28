import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement, isValidElement } from 'react';
import type { ComponentProps, ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type CountryPicker from '../../components/CountryPicker';
import type DrillHost from '../../charts/drill/DrillHost';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { DashboardTemplate } from '../../dashboard/model';
import type { ProductRequest } from '../../dashboard/client';
import type { CountryGroupReply, FuelConsumptionReply, FuelReply } from '../../generated/dashboard';
import { plotFrame } from '../../charts/frame.ts';
import { pickLines } from '../../charts/linePick.ts';
import { readFuelCountry } from './fuelCountry.ts';
import { fuelRecipientToken } from './fuelDrill.ts';
import { fuelConsumptionLevelToken, fuelConsumptionToken } from './fuelConsumption.ts';

const label = { key: 'synthetic-source', template: 'Synthetic source: {NUM}', arguments: ['NUM'] as ('NUM' | 'OIL')[], reason: null };
const fuelRead = (firstHour: number, lastHour: number, country: string, instance: number, gained: string): FuelReply['reads'][number] => ({
  firstHour, lastHour, country, countryInstance: instance, continuityStartHour: 100, startsSegment: true,
  fuelRaw: '32768', capacityRaw: '65536', lendLeaseRaw: '0', leftOut: null, awardsReason: null,
  day: { gainedRaw: gained, burnedRaw: '-65536', receivedRaw: ['0', '0', '0'], otherRaw: '0',
    awards: { gainsRaw: [gained, '0', '0', '0'], ownGainRaw: gained, ownGainQuantityRaw: '0',
      transfersRaw: ['0', '0', '0', '0'], consumedRaw: '-65536', receivedRaw: ['0', '0', '0'] } },
});
const fuelReply = (tag: string, reads: FuelReply['reads']): FuelReply => ({ branchId: '1', tag, reads, warnings: [],
  quantityScale: 32768, gainScale: 100000, dayHours: 24, detailHours: 720,
  consumerNames: ['Synthetic land', 'Synthetic air', 'Synthetic sea'], gainLabels: [label, label, label, label], transferLabels: [label, label, label] });
const catalogue: CountryGroupReply = { branchId: '1', warnings: [], humans: [], factions: [], wars: [], warStarts: [], capitulations: [],
  identityChanges: [], unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 120, slot: 2, countryId: 20 }] },
    { tag: 'Y', identities: [{ firstHour: 110, lastHour: 114, slot: 1, countryId: 10 }, { firstHour: 117, lastHour: 120, slot: 3, countryId: 30 }] },
    { tag: 'Z', identities: [{ firstHour: 100, lastHour: 120, slot: 4, countryId: 40 }] },
  ] };
const fuels = [fuelReply('X', [fuelRead(100, 109, '1', 10, '1100000'), fuelRead(110, 120, '2', 20, '9900000')]),
  fuelReply('Y', [fuelRead(110, 114, '1', 10, '1300000'), fuelRead(117, 120, '3', 30, '8800000')]),
  fuelReply('Z', [fuelRead(100, 120, '4', 40, '7700000')])];
const consumptions: FuelConsumptionReply[] = fuels.map(reply => ({ branchId: '1', tag: reply.tag, warnings: [], reads: reply.reads.map(read => ({
  firstHour: read.firstHour, lastHour: read.lastHour, startsSegment: true, segmentStartHour: read.firstHour,
  country: read.country, countryInstance: read.countryInstance, nativeDailyRaw: [read.day!.gainedRaw, '0', '0'],
  knownDailyRaw: [read.day!.gainedRaw, '0', '0'], positiveDailyRaw: [read.day!.gainedRaw, '0', '0'], reasons: [null, null, null],
})) }));

function findButton(node: ReactNode, label: string): ReactElement<{ onClick: () => void }> | null {
  if (Array.isArray(node)) return node.map(child => findButton(child, label)).find(Boolean) ?? null;
  if (!isValidElement<{ children?: ReactNode; onClick?: () => void }>(node)) return null;
  if (node.type === 'button' && node.props.children === label) return node as ReactElement<{ onClick: () => void }>;
  return findButton(node.props.children, label);
}

test('the mounted Fuel country chooser pins its original actor and restores Follow and complete alias history', async () => {
  const state = { hooks: [] as unknown[], cursor: 0, changed: false, effects: [] as (() => void)[], hour: 105, pending: false, groups: catalogue as CountryGroupReply | undefined,
    groupError: undefined as string | undefined, requests: [] as ProductRequest[], added: [] as WidgetSpec[], headerReads: [] as { tag: string; hour: number }[],
    slot: { id: 'country-a' } as unknown as HTMLElement, summarySlot: { id: 'summary' } as unknown as HTMLElement,
    bodyHeaderSlot: null as HTMLElement | null, bodyCountrySlot: null as HTMLElement | null,
    portals: [] as { slot: HTMLElement; chip: ReactElement<{ label?: { name: string } | null; hour?: number | null; reason?: string | null }> }[],
    nameKnown: true as boolean | undefined, nameFailed: false, headerName: undefined as string | undefined,
    flagReason: undefined as string | undefined,
    flagPending: false,
    main: undefined as CountryRaceViewProps | undefined, picker: undefined as ComponentProps<typeof CountryPicker> | undefined,
    host: undefined as ComponentProps<typeof DrillHost> | undefined, charts: [] as TimeChartProps[],
    segment: undefined as { onChange: (value: 'gain' | 'burn' | 'consumption') => void } | undefined };
  const globals = globalThis as typeof globalThis & { __fuelCountryMount?: typeof state };
  globals.__fuelCountryMount = state;
  // Retain the actual widget's hook cells for one mount, then discard them on
  // unmount. Transport, canvas and popover placement are the only replaced leaves.
  const hooks = `const cell = init => { const state = globalThis.__fuelCountryMount, index = state.cursor++; return state.hooks[index] ??= init(); };
    export const useEffect = (make, deps) => { const saved = cell(() => ({ deps: undefined, cleanup: undefined }));
      if (!saved.deps || saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) {
        saved.deps = deps; globalThis.__fuelCountryMount.effects.push(() => { saved.cleanup?.(); saved.cleanup = make(); });
      } };
    export const useRef = initial => cell(() => ({ current: initial }));
    export const useMemo = (make, deps) => { const saved = cell(() => ({ deps, value: make() }));
      if (saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) { saved.deps = deps; saved.value = make(); }
      return saved.value; };
    export const useCallback = (callback, deps) => useMemo(() => callback, deps);
    export const useState = initial => { const saved = cell(() => ({ value: typeof initial === 'function' ? initial() : initial }));
      return [saved.value, next => { const value = typeof next === 'function' ? next(saved.value) : next;
        if (!Object.is(value, saved.value)) globalThis.__fuelCountryMount.changed = true; saved.value = value; }]; };
    export const useReducer = (reduce, initial) => { const saved = cell(() => ({ value: initial }));
      return [saved.value, action => { const value = reduce(saved.value, action);
        if (!Object.is(value, saved.value)) globalThis.__fuelCountryMount.changed = true; saved.value = value; }]; };`;
  const replacements: Record<string, string> = {
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    // The body is called outside React, so outside a widget frame: its errors stay its own.
    '/dashboard/heldFailures.ts': `import { createContext } from 'react'; const none = [];
      export class HeldFailures { subscribe = () => () => {}; read = () => none; retry() {} }
      export const HeldFailuresContext = createContext(null); export const useHeldFailure = error => error;
      export const useHeldFailures = () => false;`,
    '/dashboard/parityPreparation.ts': 'export const useParityPreparation = () => ({ requested: false, prepare() {} });',
    '/dashboard/products.ts': `const fuels = ${JSON.stringify(fuels)}, consumptions = ${JSON.stringify(consumptions)};
      export function useProducts(_client, requests, active, validate) {
        const state = globalThis.__fuelCountryMount; if (active) state.requests.push(...requests);
        if (!requests.length || state.pending) return { retry() {} };
        const inventory = requests[0].product === 'country-fuel-consumption' ? consumptions : fuels;
        return { data: requests.map(request => validate(inventory.find(reply => reply.tag === request.params.tag))), retry() {} };
      }
      export function useProduct(_client, request, active, validate) {
        const state = globalThis.__fuelCountryMount;
        if (!active || !request) return { retry() {} }; state.requests.push(request);
        if (request.product === 'country-fuel-awards') return { data: validate({ branchId: '1', tag: request.params.tag, hour: request.params.hour,
          warnings: [], awards: { totals: fuels[0].reads[0].day.awards, recipients: [{ key: 'original-recipient', kind: 0, reference: '7', owner: '1', host: '0',
            firstHour: 100, lastHour: 105, amountRaw: '0', recordedName: 'Original recipient', nameReason: null }] } }), retry() {} };
        const summary = consumptions.find(reply => reply.tag === request.params.tag), read = summary.reads.find(read => read.firstHour === request.params.hour);
        return { data: validate({ branchId: '1', tag: request.params.tag, hour: request.params.hour, read,
          parts: [{ key: 'original-occurrence', kind: 0, reference: '7', parent: '0', ordinal: 0, owners: ['1', '1', '1'],
            wingOwner: null, group: null, topologyReason: null, site: null, amountRaw: read.knownDailyRaw[0], reason: null,
            recordedName: 'Original occurrence', nameReason: null }], commands: [], fleets: [], sites: [], topologyReason: null, warnings: [] }), retry() {} };
      }
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`,
    '/dashboard/productWindows.ts': `export function useProductWindows(_client, _identity, request, wanted, active, validate) {
      const state = globalThis.__fuelCountryMount; if (active) for (const window of wanted) state.requests.push(request(window));
      const call = request(0);
      if (call.product === 'country-fuel-demand-history') return { data: new Map([[0, validate({ ...call.params, country: '1', countryInstance: 10,
        segmentStartHour: 100, firstHour: 0, lastHour: 719, runs: [{ firstHour: 100, lastHour: 114, startsSegment: true,
          amountRaw: '1300000', reason: null, recordedName: 'Original occurrence', nameReason: null }], warnings: [] })]]), errors: new Map(), retry() {} };
      return { data: new Map([[0, validate({ branchId: '1', tag: call.params.tag, hour: call.params.hour, country: '1',
        countryInstance: 10, continuityStartHour: 100, recipientKey: call.params.recipientKey, window: 0, firstHour: 0, lastHour: 719,
        runs: [{ firstHour: 100, lastHour: 109, startsSegment: true, amountRaw: '0', reason: null }], warnings: [] })]]), errors: new Map(), retry() {} }; }`,
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active) => {
      const state = globalThis.__fuelCountryMount; return { data: active ? state.groups : undefined, error: active ? state.groupError : undefined, retry() {} }; };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__fuelCountryMount.hour; export const usePlaybackSpeed = () => 24;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ accent: '#123', muted: '#888', categories: ['#123', '#456'] });
      export const useViewportHeight = () => 1000;`,
    '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__fuelCountryMount.main = props; return props.state ?? createElement('div', null, props.beforeChart); };`,
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__fuelCountryMount.picker = props; return null; };',
    '/components/Segmented.tsx': 'export const Segmented = props => { globalThis.__fuelCountryMount.segment = props; return null; };',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/dashboard/CountryChip.tsx': 'export default () => "DEFAULT_COUNTRY_CHIP";',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      const state = globalThis.__fuelCountryMount; state.headerReads = wanted; return { labelAt: hour => tag => ({
        name: state.headerName ?? tag + ' shown at ' + hour, known: state.nameKnown, nameFailed: state.nameFailed,
        flag: state.flagReason === undefined && !state.flagPending ? 'data:flag-' + tag + '@' + hour : undefined,
        flagPending: state.flagPending, flagReason: state.flagReason }) }; };`,
    '/dashboard/WidgetSettingsMenu.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__fuelCountryMount.charts.push(props); return null; };',
    '/fuel/FuelConsumptionStack.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__fuelCountryMount.host = props; return createElement('div', null, props.main, props.open ? props.children : null); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'fuel-country-mount', enforce: 'pre',
      resolveId(id) { if (id === 'fuel-country-hooks') return '\0fuel-country-hooks'; if (id === 'react-grid-layout') return '\0fuel-country-grid';
        if (id === 'fuel-country-portals') return '\0fuel-country-portals'; },
      load(id) { if (id === '\0fuel-country-hooks') return hooks;
        if (id === '\0fuel-country-portals') return `import { createElement } from 'react'; export const createPortal = (chip, slot) => {
          globalThis.__fuelCountryMount.portals.push({ chip, slot }); return createElement('span', { 'data-header-target': slot.id }, chip); };`;
        if (id === '\0fuel-country-grid') return `export const useContainerWidth = () => ({ width: 1200, containerRef: null, mounted: true });
          export const Responsive = ({ children }) => children;`; },
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/dashboard/DashboardGrid.tsx')) return source
          .replace('const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);',
            'const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(globalThis.__fuelCountryMount.summarySlot);')
          .replace('const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);',
            'const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(globalThis.__fuelCountryMount.slot);');
        if (path.endsWith('/fuel/FuelWidget.tsx') || path.endsWith('/fuel/preparedFuel.ts') || path.endsWith('/dashboard/viewRead.ts')
          || path.endsWith('/dashboard/comparisonCountries.ts') || path.endsWith('/dashboard/useDiagnostics.ts'))
          return source.replaceAll("from 'react'", "from 'fuel-country-hooks'").replace("from 'react-dom'", "from 'fuel-country-portals'");
        if (path.endsWith('/supply-health/SupplyHealthWidget.tsx')) return source.replace("from 'react-dom'", "from 'fuel-country-portals'");
      },
    }] });
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    registry.registerWidgets([{ ...(await vite.ssrLoadModule('/ui/src/widgets/fuel/definition.ts')).definition,
      ...(await vite.ssrLoadModule('/ui/src/widgets/fuel/index.ts')).default }]);
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/fuel/FuelWidget.tsx')).default as (props: WidgetProps) => ReactElement;
    const widget = { id: 'fuel-country', kind: 'fuel', configVersion: 1, title: 'Fuel', trackIds: ['track'],
      countries: { kind: 'tags', tags: ['X', 'Z'] }, time: { mode: 'campaign' }, options: {} } as WidgetSpec;
    const props = { widget, active: true, onDiagnostics() {}, runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0,
      branch: { id: '1', firstHour: 100, lastHour: 120 }, source: { id: 'synthetic', revision: '1', path: 'synthetic.battleplan', metadata: { campaignKey: 'c1', countries: [] } } }],
      clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) },
      addWidgetBelow: (_id: string, pinned: WidgetSpec) => state.added.push(pinned) } } as unknown as WidgetProps;
    let element: ReactElement;
    const unmount = () => { for (const cell of state.hooks as { cleanup?: () => void }[]) cell.cleanup?.(); state.hooks = []; };
    const render = () => { for (;;) {
      state.changed = false; state.cursor = 0; state.requests = []; state.charts = []; state.picker = undefined; state.host = undefined;
      state.portals = []; state.headerReads = [];
      element = Widget(props); const html = renderToStaticMarkup(element);
      const effects = state.effects; state.effects = []; for (const effect of effects) effect();
      if (!state.changed) return html;
    } };
    render();
    assert.deepEqual(state.requests.filter(request => request.product === 'country-fuel').map(request => request.params),
      [{ branchId: '1', tag: 'X' }, { branchId: '1', tag: 'Z' }], 'MAIN comparison requests stay literal');
    const originalPick = state.main!.onPick!;
    const frame = plotFrame(400, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [100, 120], [0, 2], false);
    const choices = pickLines(state.main!.lines.map(line => ({ key: line.tag, runs: line.runs, held: false })), 105,
      { frame, until: null, point: { x: frame.toX(105), y: frame.toY(1) } });
    assert.deepEqual(choices, ['X', 'Z']);
    state.main!.onPick!(105, undefined, choices); render();
    assert.deepEqual(state.picker!.allowedTags, choices);
    const obsoleteChooser = state.picker!.onChange;
    state.picker!.onChange('X'); render();
    state.segment!.onChange('gain');
    assert.match(render(), /Synthetic source: 11/);
    state.host!.onPin!();
    const pinned = state.added[0];
    const pinnedOptions = pinned.options!;
    assert.deepEqual(readFuelCountry(pinnedOptions.fuelCountry, 'c1', '1'), { campaign: 'c1', branch: '1', tag: 'X', originOffset: 5 });
    assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['X'] });
    assert.deepEqual(pinned.time, { mode: 'pin', windows: { track: { firstOffset: 5, lastOffset: 5 } } });
    obsoleteChooser('Z'); render(); assert.match(render(), /Synthetic source: 11/);
    const abandonedPin = state.host!.onPin!;
    unmount(); props.widget = pinned; state.hour = 112; // Actual saved spec, with all mounted state discarded.
    abandonedPin(); assert.equal(state.added.length, 1, 'unmount cleanup revokes the prior Pin callback');
    assert.match(render(), /Synthetic source: 11/);
    findButton(element!, 'Follow playback')!.props.onClick();
    const followed = render();
    assert.match(followed, /Synthetic source: 13/);
    assert.doesNotMatch(followed, /Synthetic source: 99|Synthetic source: 88/);
    const chart = state.charts.find(chart => chart.label === 'Y: own fuel production over time')!;
    const parts = chart.revision as BreakdownSeries[];
    const source = parts.find(part => part.id === 'gain-0')!;
    assert.deepEqual(source.runs.map(run => [run.firstHour, run.lastHour, run.value]), [[100, 109, 11], [110, 114, 13]]);
    assert.equal(source.amount(105), '11'); assert.equal(source.amount(112), '13'); assert.equal(source.amount(119), 'Not recorded');
    const owner = chart.pickOwner;
    state.hour = 113; render(); assert.equal(state.charts[0].pickOwner, owner, 'ordinary Follow keeps its numeric owner');
    chart.onPick!(104); assert.match(render(), /Synthetic source: 11/);
    assert.deepEqual(state.charts[0].extent, [100, 120], 'history date picks keep the saved extent');
    findButton(element!, 'Follow playback')!.props.onClick(); state.hour = 116;
    assert.match(render(), /Country unavailable at this time/);
    state.hour = 119; assert.doesNotMatch(render(), /Synthetic source: 99|Synthetic source: 88/);
    const Grid = (await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx')).default;
    registry.registerWidgets([{ ...(await vite.ssrLoadModule('/ui/src/widgets/fuel/definition.ts')).definition,
      ...(await vite.ssrLoadModule('/ui/src/widgets/fuel/index.ts')).default, render: (bodyProps: WidgetProps) => {
        state.bodyHeaderSlot = bodyProps.headerSlot; state.bodyCountrySlot = bodyProps.countrySlot ?? null; return null;
      } }]);
    const longTitle = 'A long saved Fuel title repeated without room '.repeat(12);
    const template = { version: 1, name: 'Synthetic', alignment: 'calendar', roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 'track', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: null }], widgets: [{ ...pinned, title: longTitle, campaign: 'tracks' }], layouts: {} } as DashboardTemplate;
    const headerRuntime = { ...props.runtime, campaignTracks: template.tracks, defaultRoleId: 'primary', maps: { schedule() {} } };
    const header = () => renderToStaticMarkup(createElement(Grid, { template, runtime: headerRuntime, editing: false, onLayouts() {}, onApply() {}, onRemove() {} }));
    const pinnedFrame = header();
    assert.doesNotMatch(pinnedFrame, /DEFAULT_COUNTRY_CHIP/);
    assert.match(pinnedFrame, /<\/h2>(?:<span class="dash-widget-subtitle">[^<]*<\/span>)?<\/div><button[^>]*class="dash-widget-country-button"[^>]*><span class="dash-widget-country-slot"><\/span><svg[^>]*class="dash-widget-country-chevron"/,
      'the chip target follows the long title and is outside its clipped h2');
    assert.equal(state.bodyCountrySlot, state.slot, 'actual Grid and WidgetBody hand the separate country slot to the registered body');
    assert.equal(state.bodyHeaderSlot, state.summarySlot, 'the existing title-summary slot remains independent');
    template.widgets = [{ ...widget, campaign: 'tracks' }]; const mainFrame = header();
    assert.match(mainFrame, /DEFAULT_COUNTRY_CHIP/, 'MAIN keeps the shared default country chip');
    assert.doesNotMatch(mainFrame, /dash-widget-country-slot/);
    registry.registerWidgets([{ ...(await vite.ssrLoadModule('/ui/src/widgets/supply-health/definition.ts')).definition,
      ...(await vite.ssrLoadModule('/ui/src/widgets/supply-health/index.ts')).default }]);
    template.widgets = [{ ...widget, kind: 'supply-health', campaign: 'tracks', trackIds: ['unloaded'] }];
    state.portals = []; const supplyFrame = header();
    assert.match(supplyFrame, /share of divisions/);
    assert.match(supplyFrame, /DEFAULT_COUNTRY_CHIP/, 'actual Supply MAIN retains its default chip beside its title summary');
    assert.doesNotMatch(supplyFrame, /dash-widget-country-slot/);
    assert.deepEqual(state.portals.map(portal => portal.slot), [state.summarySlot], 'actual Supply summary still portals only into headerSlot');
    const currentPin = (fuelCountry: unknown) => ({ ...pinned, campaign: 'tracks',
      time: { mode: 'pin', windows: { track: { firstOffset: 12, lastOffset: 12 } } },
      options: { ...pinnedOptions, fuelCountry } }) as unknown as WidgetSpec;
    state.hour = 112;
    // Numbers/booleans are stored-option forms; the array is a direct-prop guard.
    for (const raw of [17, true, ['invalid-direct-prop'], '']) {
      const legacy = currentPin(raw); template.widgets = [legacy]; props.widget = legacy; unmount();
      const frame = header();
      assert.match(frame, /DEFAULT_COUNTRY_CHIP/, 'non-string or empty country tokens keep the legacy frame chip');
      assert.doesNotMatch(frame, /dash-widget-country-slot/);
      props.countrySlot = state.bodyCountrySlot;
      assert.match(render(), /Synthetic source: 99/, 'the actual body retains the matching legacy literal-tag semantics');
      assert.deepEqual(state.portals, []); assert.deepEqual(state.headerReads, []);
    }
    const malformed = currentPin('malformed-native-selection'); template.widgets = [malformed]; props.widget = malformed; unmount();
    assert.doesNotMatch(header(), /DEFAULT_COUNTRY_CHIP/);
    props.countrySlot = state.bodyCountrySlot;
    const refused = render();
    assert.match(refused, /Unavailable/);
    assert.match(refused, /Choose a country again/);
    assert.equal(state.portals.length, 1, 'a malformed nonempty string keeps the explicit neutral refusal chip');
    assert.deepEqual(state.headerReads, []);
    template.widgets = [{ ...pinned, campaign: 'tracks' }];
    header(); props.headerSlot = state.bodyHeaderSlot; props.countrySlot = state.bodyCountrySlot;
    props.widget = { ...pinned, time: { mode: 'pin', windows: { track: { firstOffset: 12, lastOffset: 12 } } } }; unmount();
    assert.match(render(), /Y shown at 112/);
    assert.deepEqual(state.headerReads, [{ tag: 'Y', hour: 112 }]);
    assert.equal(state.portals[0].slot, state.slot);
    assert.equal(state.portals[0].chip.props.hour, 112);
    assert.match(render(), /data:flag-Y@112/);
    state.charts[0].onPick!(115);
    assert.match(render(), /Unavailable/);
    assert.equal(state.portals[0].chip.props.hour, 115, 'a local history pick changes the header date even while the saved spec is Pin 12');
    assert.deepEqual(state.headerReads, [], 'a native identity gap never requests the tag replacement');
    assert.deepEqual(props.widget.time, { mode: 'pin', windows: { track: { firstOffset: 12, lastOffset: 12 } } });
    state.charts[0].onPick!(113); assert.match(render(), /Y shown at 113/);
    state.hour = 114; findButton(element!, 'Follow playback')!.props.onClick(); assert.match(render(), /Y shown at 114/);
    assert.equal(state.portals[0].chip.props.hour, 114);
    assert.deepEqual(props.widget.time, { mode: 'pin', windows: { track: { firstOffset: 12, lastOffset: 12 } } });
    state.hour = 119; assert.match(render(), /Unavailable/); assert.deepEqual(state.headerReads, []);
    state.hour = 112; render();
    state.slot = { id: 'country-b' } as unknown as HTMLElement; header(); props.countrySlot = state.bodyCountrySlot;
    assert.match(render(), /data-header-target="country-b"/);
    assert.equal(state.portals[0].slot, state.slot, 'country slot replacement moves the local chip without replacing the native actor');
    state.nameKnown = false; assert.match(render(), /Unavailable/);
    assert.match(render(), /data:flag-Y@112/, 'the authenticated current flag stays available when its name is missing');
    state.flagReason = 'Synthetic dated flag refusal';
    assert.match(render(), /Name unavailable/);
    assert.match(render(), /aria-label="Flag unavailable"/);
    assert.doesNotMatch(render(), /data:flag-Y@112/);
    state.flagReason = undefined;
    state.nameKnown = undefined; assert.match(render(), /Loading/);
    assert.match(render(), /data:flag-Y@112/, 'pending names do not hide an independently authenticated flag');
    state.nameKnown = true; state.nameFailed = true; assert.match(render(), /Name unavailable/);
    assert.match(render(), /data:flag-Y@112/, 'failed names do not hide an independently authenticated flag');
    state.flagPending = true; assert.match(render(), /aria-label="Flag loading"/);
    state.flagPending = false;
    state.flagReason = 'The flag request failed.';
    assert.match(render(), /Name unavailable/);
    assert.match(render(), /aria-label="Flag unavailable"/);
    assert.doesNotMatch(render(), /data:flag-Y@112/);
    state.flagReason = undefined;
    state.nameFailed = false; state.headerName = ''; assert.doesNotMatch(render(), /Unavailable/);
    state.headerName = undefined; props.countrySlot = null; render(); assert.deepEqual(state.portals, []); assert.deepEqual(state.headerReads, []);
    props.countrySlot = state.slot;
    state.hour = 112; props.widget = { ...pinned, time: { mode: 'follow' }, options: { ...pinnedOptions, show: 'burn' } }; unmount();
    assert.match(render(), /Fuel burned.*?−2/s);
    props.widget = { ...pinned, time: { mode: 'follow' }, options: { ...pinnedOptions, show: 'consumption' } }; unmount();
    assert.match(render(), /demand per day.*?13/s);
    const families = state.charts.find(chart => chart.label === 'Y: current consumption families')!.revision as BreakdownSeries[];
    assert.deepEqual(families[0].runs.map(run => [run.firstHour, run.lastHour, run.value]), [[100, 109, 11], [110, 114, 13]]);
    const consumptionRequest = state.requests.find(request => request.product === 'country-fuel-demand')!;
    assert.deepEqual(consumptionRequest.params, { branchId: '1', tag: 'Y', hour: 110, country: '1', countryInstance: 10 });
    const originalBasis = { branchId: '1', tag: 'X', hour: 100, country: '1', countryInstance: 10 };
    props.widget = { ...pinned, time: { mode: 'follow' }, options: { ...pinnedOptions, show: 'burn', fuelRecipient: fuelRecipientToken({ campaign: 'c1', branch: '1',
      consumer: 0, basis: originalBasis, key: 'original-recipient', continuityStartHour: 100 }) } }; unmount();
    assert.match(render(), /Original recipient/);
    assert.deepEqual(state.requests.find(request => request.product === 'country-fuel-awards')!.params, originalBasis);
    assert.deepEqual(state.requests.find(request => request.product === 'country-fuel-award-history')!.params,
      { ...originalBasis, window: 0, recipientKey: 'original-recipient' });
    props.widget = { ...pinned, time: { mode: 'follow' }, options: { ...pinnedOptions, show: 'consumption',
      fuelConsumption: fuelConsumptionToken({ campaign: 'c1', branch: '1', basis: originalBasis, key: 'original-occurrence', segmentStartHour: 100 }),
      fuelConsumptionLevel: fuelConsumptionLevelToken({ campaign: 'c1', branch: '1', tag: 'X', country: '1', countryInstance: 10,
        segmentStartHour: 100, level: 'family:0' }) } }; unmount();
    const occurrence = render();
    assert.match(occurrence, /Original occurrence/);
    assert.match(occurrence, /Y shown at 112/);
    assert.doesNotMatch(occurrence, /Choose a country again/);
    assert.deepEqual(state.requests.find(request => request.product === 'country-fuel-demand-history')!.params,
      { ...originalBasis, window: 0, key: 'original-occurrence' });
    props.widget = { ...props.widget, options: { ...props.widget.options, fuelConsumption: '' } }; unmount();
    assert.match(render(), /Choose a country again/, 'a held level still retains its own segment restriction');
    props.widget = pinned; unmount(); state.groups = undefined;
    assert.doesNotMatch(header(), /DEFAULT_COUNTRY_CHIP/);
    assert.match(render(), /Loading country/);
    assert.match(render(), /Unavailable/); assert.deepEqual(state.headerReads, []);
    assert.deepEqual(state.requests.map(request => request.params), [{ branchId: '1', tag: 'X' }], 'an unauthenticated origin never asks for alias values');
    state.groups = { ...catalogue, countryIdentities: [] }; unmount();
    assert.match(render(), /Choose a country again/);
    state.groups = { ...catalogue, branchId: 'other' }; unmount();
    assert.match(render(), /Choose a country again/);
    assert.doesNotMatch(header(), /DEFAULT_COUNTRY_CHIP/); assert.deepEqual(state.headerReads, []);
    state.groups = catalogue; state.groupError = 'Synthetic native catalogue refusal'; unmount();
    const refreshFailed = render();
    assert.match(refreshFailed, /Synthetic native catalogue refusal/);
    assert.match(refreshFailed, />Retry</);
    assert.match(refreshFailed, /Synthetic source: 11/, 'the pinned breakdown keeps its data beside the failure');
    state.groups = undefined; unmount();
    const unloaded = render();
    assert.match(unloaded, /Synthetic native catalogue refusal/, 'without published identities the failure stands alone');
    assert.doesNotMatch(unloaded, /Synthetic source: 11/);
    state.groups = catalogue; unmount();
    state.groupError = undefined; unmount(); state.hour = 105; render();
    const firstOwner = state.charts[0].pickOwner;
    state.pending = true; assert.match(render(), /Fuel gained \(own production\)/);
    assert.equal(state.charts[0].pickOwner, firstOwner, 'content pending retains identical numbers and actor ownership');
    state.pending = false; unmount(); props.widget = widget; render();
    const client = props.runtime.client; props.runtime.client = {} as never; render(); props.runtime.client = client; render();
    const count = state.added.length; originalPick(105, 'X', ['X']); render();
    assert.equal(state.host!.open, false, 'A to B to A clients cannot revive an obsolete drawn-country pick');
    assert.equal(state.added.length, count);

    const preparedRequests: { product: string; params: { reads?: { tag: string; hour: number }[]; tags?: string[]; measures?: string[]; from?: number; to?: number; width?: number } }[] = [];
    props.runtime.client = { async preparedQuery(_source: unknown, product: string, params: typeof preparedRequests[number]['params']) {
      preparedRequests.push({ product, params });
      const tags = [...new Set(params.reads?.map(read => read.tag) ?? params.tags ?? [])];
      const replies = fuels.filter(reply => tags.includes(reply.tag)).map(reply => ({ ...reply, reads: [] }));
      if (product === 'series-state') return { branchId: '1', replies, states: params.reads!.map(({ tag, hour }) => ({
        tag, hour, recorded: true,
        fuel: fuels.find(reply => reply.tag === tag)?.reads.find(read => read.firstHour <= hour && read.lastHour >= hour) ?? null,
        consumption: consumptions.find(reply => reply.tag === tag)?.reads.find(read => read.firstHour <= hour && read.lastHour >= hour) ?? null,
        consumptionSegment: { firstHour: 100, lastHour: 114 },
      })) };
      assert.equal(product, 'series-summary');
      return { branchId: '1', replies, series: tags.flatMap(tag => params.measures!.map(measure => ({ tag, measure,
        summary: { from: params.from, to: params.to, width: params.width, buckets: [], runs: [] },
      }))) };
    } } as unknown as typeof props.runtime.client;
    const settlePrepared = async () => {
      for (let turn = 0; turn < 4; turn++) {
        await new Promise<void>(resolve => setImmediate(resolve));
        render();
        assert.equal(state.requests.some(request => ['country-fuel', 'country-fuel-consumption'].includes(request.product)), false,
          'a prepared mount never requests a whole-branch fuel or consumption reply');
      }
    };
    props.widget = widget; state.hour = 105; unmount(); render(); await settlePrepared();
    assert.equal(preparedRequests[0].product, 'series-state');
    state.main!.onSummaryView!({ from: 100, to: 105, width: 200 }); render(); await settlePrepared();
    assert(preparedRequests.some(request => request.product === 'series-summary' && request.params.width === 200));
    props.widget = { ...pinned, time: { mode: 'follow' }, options: { ...pinnedOptions, show: 'consumption' } };
    unmount(); render(); await settlePrepared();
    const preparedFamilies = state.charts.find(chart => chart.label === 'X: current consumption families')!;
    assert(preparedFamilies.onSummaryView, 'the mounted consumption breakdown requests shared summaries');
    preparedFamilies.onSummaryView!({ from: 100, to: 105, width: 160 }); render(); await settlePrepared();
    assert(preparedRequests.some(request => request.product === 'series-summary' && request.params.measures?.includes('consumption-0')));
    assert(state.requests.some(request => request.product === 'country-fuel-demand'), 'current recipient details still use the bounded detail product');
    unmount();
  } finally { await vite.close(); delete globals.__fuelCountryMount; }
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { createWidget } from '../../dashboard/model.ts';
import { setOption } from '../../dashboard/options.ts';
import { CLOSED, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ProductRequest } from '../../dashboard/client';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { plotFrame } from '../../charts/frame.ts';
import { tooltipHeight, tooltipRoom, tooltipText } from '../../charts/tooltip.ts';
import type { DrillAction, DrillState } from '../../charts/drill/drillState';
import type { DrillPart } from '../../charts/drill/parts';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { CountryRaceLine } from '../../charts/race/countryRace';
import type { BranchMetadata, FuelAwardsReply, FuelAwardHistoryReply, FuelConsumptionReply, FuelDemandReply, FuelDemandHistoryReply, FuelDemandStackReply, FuelLabel, FuelReply } from '../../generated/dashboard';
import '../definitions.ts';
import { BREAKDOWNS, OPTIONS } from './definition.ts';
import { fuelExtentToken, fuelRecipientToken, readFuelRecipient } from './fuelDrill.ts';
import type { FuelRecipientSelection } from './fuelDrill';
import { fuelConsumptionToken, readFuelConsumption, readFuelConsumptionLevel } from './fuelConsumption.ts';
import { formatFuelPart } from './fuelData.ts';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from './fuelConsumption';

const label = (key: string): FuelLabel => ({ key, template: `${key}: {NUM}`, arguments: ['NUM'], reason: null });
const branch: BranchMetadata = { id: '1', sessionId: '1', firstHour: 0, lastHour: 200,
  parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] };
const reply: FuelReply = { branchId: '1', tag: 'AAA', quantityScale: 32768, gainScale: 100000, dayHours: 24, detailHours: 720,
  gainLabels: [label('Synthetic base'), label('Synthetic states'), label('Synthetic oil'), label('Synthetic flat gain')],
  transferLabels: [label('Synthetic incoming'), label('Synthetic outgoing'), label('Synthetic flat cost')],
  consumerNames: ['Synthetic land', 'Synthetic air', 'Synthetic sea'], warnings: [], reads: [{
    firstHour: 100, lastHour: 123, startsSegment: true, country: '7', countryInstance: 9, continuityStartHour: 77,
    fuelRaw: '32768', capacityRaw: '65536', lendLeaseRaw: '0', leftOut: null, awardsReason: null,
    day: { gainedRaw: '100000', burnedRaw: '32768', receivedRaw: ['100000', '0', '0'], otherRaw: '0',
      awards: { gainsRaw: ['100000', '0', '0', '0'], ownGainRaw: '100000', ownGainQuantityRaw: '32768',
        transfersRaw: ['0', '0', '0', '0'], consumedRaw: '32768', receivedRaw: ['100000', '0', '0'] } },
  }] };
const awards: FuelAwardsReply = { branchId: '1', tag: 'AAA', hour: 100, warnings: [], awards: {
  totals: reply.reads[0].day!.awards!, recipients: [{ key: 'opaque-key', kind: 0, reference: '9007199254741000', owner: '7', host: '0',
    firstHour: 77, lastHour: 100, amountRaw: '100000', recordedName: 'Recorded recipient A', nameReason: null }],
} };
const history: FuelAwardHistoryReply = { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9,
  continuityStartHour: 77, window: 0, recipientKey: 'opaque-key', firstHour: 0, lastHour: 719, warnings: [],
  runs: [{ firstHour: 100, lastHour: 123, startsSegment: true, amountRaw: '100000', reason: null }] };
const consumption: FuelConsumptionReply = { branchId: '1', tag: 'AAA', warnings: [], reads: [{ firstHour: 100, lastHour: 123,
  startsSegment: true, segmentStartHour: 100, country: '7', countryInstance: 9, nativeDailyRaw: ['240000', '-100000', '0'],
  knownDailyRaw: ['240000', '-100000', '0'], positiveDailyRaw: ['240000', '0', '0'], reasons: [null, null, null] }] };
const demand: FuelDemandReply = { branchId: '1', tag: 'AAA', hour: 100, read: consumption.reads[0], commands: [], fleets: [], sites: [], warnings: [], topologyReason: null,
  parts: [{ key: 'current-native-key', kind: 0, reference: '9007199254741001', parent: '0', ordinal: 0, owners: ['7', '7', '7'],
    wingOwner: null, group: null, topologyReason: null, site: null, amountRaw: '240000', reason: null, recordedName: 'Current recorded division', nameReason: null },
    { key: 'current-wing-key', kind: 1, reference: '9007199254741002', parent: '0', ordinal: 0, owners: null, wingOwner: '7', group: null,
      topologyReason: null, site: null, amountRaw: '-100000', reason: null, recordedName: 'Current recorded wing', nameReason: null }] };
const demandHistory: FuelDemandHistoryReply = { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9, segmentStartHour: 100,
  window: 0, key: demand.parts[0].key, firstHour: 0, lastHour: 719, warnings: [],
  runs: [{ firstHour: 100, lastHour: 123, startsSegment: true, amountRaw: '240000', reason: null, recordedName: 'Current recorded division', nameReason: null }] };
const demandStack: FuelDemandStackReply = { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9, segmentStartHour: 100,
  window: 0, firstHour: 0, lastHour: 719, warnings: [], parts: demand.parts.map(part => ({ key: part.key,
    runs: [{ firstHour: 100, lastHour: 123, startsSegment: true, amountRaw: part.amountRaw, reason: null, recordedName: part.recordedName, nameReason: null }] })) };
const host = globalThis as typeof globalThis & {
  __fuelPinRequests?: ProductRequest[]; __fuelRetainedRecipient?: FuelRecipientSelection | null; __fuelDrillState?: DrillState;
  __fuelFixture?: { reply?: FuelReply; awards?: FuelAwardsReply; consumption?: FuelConsumptionReply; demand?: FuelDemandReply; demandStack?: FuelDemandStackReply; error?: string; awardsError?: string; demandError?: string };
  __fuelNumberStore?: (client: object, kind: 'main' | 'awards') => unknown;
  __fuelPlot?: { lines: CountryRaceLine[]; pickOwner: { scope: string; numbers?: FuelReply[] }; onPick: (hour: number, tag?: string) => void };
  __fuelCharts?: TimeChartProps[]; __fuelHistoryIdentities?: string[];
  __fuelAction?: DrillAction; __fuelConsumption?: FuelConsumptionSelection | null;
  __fuelLevel?: FuelConsumptionLevel | null;
  __fuelParts?: { parts: readonly DrillPart[]; total?: number | null; onPick?: (key: string) => void }[];
  __fuelDrillHost?: { onPin?: () => void; onFollow: () => void };
};

// Actual widget, breakdown, options and Pin factory, with transport/canvas leaves and retained-state seeds below.
async function fuelFixture() {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'fuel-pin-settings-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts')) return source.slice(0, source.indexOf('export function useProducts<T>')) + `
          export function useProducts(_client, requests, active, validate) {
            if (active) globalThis.__fuelPinRequests?.push(...requests);
            if (!requests.length) return { retry() {} };
            const fixture = globalThis.__fuelFixture;
            const data = requests[0].product === 'country-fuel-consumption'
              ? fixture ? fixture.consumption : ${JSON.stringify(consumption)} : fixture ? fixture.reply : ${JSON.stringify(reply)};
            return { data: data ? [validate(data)] : undefined, error: fixture?.error, retry() {} };
          }
          export function useProduct(_client, request, active, validate) {
            if (!request || !active) return { retry() {} };
            globalThis.__fuelPinRequests?.push(request);
            const fixture = globalThis.__fuelFixture;
            const data = request.product === 'country-fuel-consumption' ? fixture ? fixture.consumption : ${JSON.stringify(consumption)}
              : request.product === 'country-fuel-demand' ? fixture ? fixture.demand : ${JSON.stringify(demand)} : fixture ? fixture.awards : ${JSON.stringify(awards)};
            return { data: data ? validate(data) : undefined, error: request.product === 'country-fuel-demand' ? fixture?.demandError : fixture?.awardsError, retry() {} };
          }`;
        if (path.endsWith('/dashboard/productWindows.ts')) return source.slice(0, source.indexOf('export function useProductWindows<T>')) + `
          export function useProductWindows(_client, identity, request, wanted, active, validate) {
            globalThis.__fuelHistoryIdentities?.push(identity);
            if (active) for (const window of wanted) globalThis.__fuelPinRequests?.push(request(window));
            const call = request(0);
            const stack = globalThis.__fuelFixture?.demandStack ?? ${JSON.stringify(demandStack)};
            const requested = new Set(call.params.keys ?? []);
            const data = call.product === 'country-fuel-demand-stack' ? { ...stack, parts: stack.parts.filter(part => requested.has(part.key)) }
              : call.product === 'country-fuel-demand-history' ? ${JSON.stringify(demandHistory)} : ${JSON.stringify(history)};
            return { data: new Map([[0, validate(data)]]), errors: new Map(), retry() {} };
          }`;
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({ retry() {} });';
        // DOM-less SSR keeps the real picker/popover panel inline; browser portal placement remains unverified.
        if (path.endsWith('/components/Popover.tsx')) return source.replace('return portal ? createPortal(panel, document.body) : panel;', 'return panel;');
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => ({ accent: '#123', muted: '#888', categories: ['#123', '#456'] });
          export const useViewportHeight = () => 1000;`;
        if (path.endsWith('/charts/TimeChart.tsx')) return 'export default props => { globalThis.__fuelCharts?.push(props); return null; };';
        if (path.endsWith('/charts/drill/DrillParts.tsx')) return source.replace('const rows = useMemo(', 'globalThis.__fuelParts?.push({ parts, total, onPick }); const rows = useMemo(');
        if (path.endsWith('/charts/drill/DrillHost.tsx')) return source.replace("const panel = usePopover('start');", "globalThis.__fuelDrillHost = { onPin, onFollow }; const panel = usePopover('start');");
        if (path.endsWith('/charts/race/CountryRaceView.tsx')) return `import { createElement } from 'react';
          export default props => { globalThis.__fuelPlot = props; return props.state ?? createElement('p', null, 'Fuel lines: ' + props.lines.length); };`;
        // Seed the previous burn selection before the options-change effect clears it; leave eligibility/render logic untouched.
        if (path.endsWith('/fuel/FuelWidget.tsx')) return source.replace('useState<FuelRecipientSelection | null | undefined>(undefined)',
          'useState<FuelRecipientSelection | null | undefined>(globalThis.__fuelRetainedRecipient)')
          .replace('useReducer(reduceDrill, CLOSED)', 'useReducer(reduceDrill, globalThis.__fuelDrillState ?? CLOSED)')
          .replace('const [storedDrill, dispatch]', 'const [storedDrill, realDispatch]')
          .replace('const clockHour =', 'const dispatch = action => { globalThis.__fuelAction = action; realDispatch(action); }; const clockHour =')
          .replace('useState<FuelConsumptionSelection | null | undefined>(undefined)', 'useState<FuelConsumptionSelection | null | undefined>(globalThis.__fuelConsumption)')
          .replace('useState<FuelConsumptionLevel | null | undefined>(undefined)', 'useState<FuelConsumptionLevel | null | undefined>(globalThis.__fuelLevel)')
          .replace('onSelection={setConsumption}', 'onSelection={value => { globalThis.__fuelConsumption = value; setConsumption(value); }}')
          .replace('onLevel={setLevel}', 'onLevel={value => { globalThis.__fuelLevel = value; setLevel(value); }}')
          .replace('new NumericReply<FuelReply[]>()', "(globalThis.__fuelNumberStore?.(runtime.client, 'main') ?? new NumericReply<FuelReply[]>())");
        if (path.endsWith('/fuel/FuelBreakdown.tsx')) return source.replace('new NumericReply<FuelAwardsReply>()',
          "(globalThis.__fuelNumberStore?.(runtime.client, 'awards') ?? new NumericReply<FuelAwardsReply>())");
      },
    }] });
  // Pin creates widgets in Vite's module graph, independently of the Node registry.
  await vite.ssrLoadModule('/ui/src/widgets/definitions.ts');
  return vite;
}

test('consumption keeps both disclosures and distinguishes unavailable demand from a known zero', async () => {
  const vite = await fuelFixture();
  try {
    const { default: Breakdown } = await vite.ssrLoadModule('/ui/src/widgets/fuel/FuelConsumptionBreakdown.tsx');
    const track = { id: 'track', axisOffset: 0, branch, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'c1' } } };
    const props = { reply, runtime: { client: {}, tracks: [track] }, track, hour: 100, selection: null,
      onSelection() {}, level: null, onLevel() {}, extent: [100, 123], branchStart: 0, active: true, pickOwner: {} };
    const render = () => renderToStaticMarkup(createElement(Breakdown, props));
    const unavailable = { ...consumption.reads[0], nativeDailyRaw: [null, null, null],
      reasons: ['Raw land failure', 'Raw air failure', 'Raw sea failure'] } as FuelConsumptionReply['reads'][number];
    host.__fuelFixture = { consumption: { ...consumption, reads: [unavailable] }, demand: { ...demand, parts: [], read: unavailable } };
    const missing = render();
    assert.match(missing, /<summary>Armies, fleets and air bases<\/summary>/);
    assert.match(missing, /<summary>By military branch<\/summary>/);
    assert.match(missing, /Daily demand subtotal[\s\S]*?Unavailable/);
    assert.doesNotMatch(missing, /Raw (land|air|sea) failure/);
    const known = { ...consumption.reads[0], nativeDailyRaw: ['0', '0', '0'], knownDailyRaw: ['0', '0', '0'], reasons: [null, null, null] } as FuelConsumptionReply['reads'][number];
    host.__fuelFixture = { consumption: { ...consumption, reads: [known] }, demand: { ...demand, parts: [], read: known } };
    const zero = render();
    assert.match(zero, /Daily demand[\s\S]*?>0</);
    assert.doesNotMatch(zero, /Daily demand subtotal/);
  } finally {
    delete host.__fuelFixture;
    await vite.close();
  }
});

test('Pin/settings burn-to-gain renders the actual gain view and stops recipient requests while retaining the hidden token', async () => {
  const vite = await fuelFixture();
  try {
    const { default: FuelWidget } = await vite.ssrLoadModule('/ui/src/widgets/fuel/FuelWidget.tsx');
    const source = createWidget('fuel', 'track', 'fuel-pin');
    source.countries = { kind: 'tags', tags: ['AAA'] };
    const pinned = pinnedBreakdown(source, BREAKDOWNS[1], 'track');
    pinned.time = { mode: 'pin', windows: { track: { firstOffset: 100, lastOffset: 100 } } };
    pinned.options = { ...pinned.options, fuelExtent: fuelExtentToken('c1', '1', [0, 200]), fuelRecipient: fuelRecipientToken({ campaign: 'c1', branch: '1', consumer: 0,
      continuityStartHour: 77, key: 'opaque-key', basis: { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9 } }) };
    const runtime = { tracks: [{ id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'c1', countries: [], branches: [branch] } },
      branch }], client: {}, clock: { getHour: () => 100, subscribeHour: () => () => {},
        getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } } as unknown as WidgetProps['runtime'];
    const render = (widget = pinned) => {
      host.__fuelPinRequests = [];
      return renderToStaticMarkup(createElement(FuelWidget, { widget, runtime, active: true, onDiagnostics() {} }));
    };
    assert.match(render(), /Recorded recipient A/);
    assert.ok(host.__fuelPinRequests!.some(request => request.product === 'country-fuel-awards'));
    assert.ok(host.__fuelPinRequests!.some(request => request.product === 'country-fuel-award-history'));
    const changed = { ...pinned, options: setOption(OPTIONS, pinned.options, 'show', 'gain')! };
    assert.equal(changed.options.fuelRecipient, pinned.options.fuelRecipient);
    const gained = render(changed);
    assert.match(gained, /Fuel gained \(own production\)/);
    assert.match(gained, /Synthetic base: 1/);
    assert.doesNotMatch(gained, /malformed|Recorded recipient A/);
    assert.deepEqual(host.__fuelPinRequests!.map(request => request.product), ['country-fuel']);
    host.__fuelRetainedRecipient = readFuelRecipient(pinned.options.fuelRecipient, 'c1', '1');
    assert.ok(host.__fuelRetainedRecipient);
    assert.match(render(changed), /Fuel gained \(own production\)/);
    assert.deepEqual(host.__fuelPinRequests!.map(request => request.product), ['country-fuel']);
    delete host.__fuelRetainedRecipient;
    assert.match(render({ ...pinned, options: { ...pinned.options, fuelRecipient: 'not-json' } }), /Choose a country again/);
    assert.deepEqual(host.__fuelPinRequests!.map(request => request.product), ['country-fuel']);
    assert.match(render({ ...changed, options: { ...changed.options, fuelRecipient: 'not-json' } }), /Fuel gained \(own production\)/);
    assert.deepEqual(host.__fuelPinRequests!.map(request => request.product), ['country-fuel']);
  } finally { delete host.__fuelPinRequests; delete host.__fuelRetainedRecipient; await vite.close(); }
});

test('content pending and label-only recovery keep the actual Fuel plot, held recipient amounts and numeric pick owner', async () => {
  const vite = await fuelFixture();
  // SSR continues the real per-client NumericReply stores and seeded held state; production projection/render logic stays intact.
  const stores = new WeakMap<object, { main: NumericReply<FuelReply[]>; awards: NumericReply<FuelAwardsReply> }>();
  host.__fuelNumberStore = (client, kind) => {
    let store = stores.get(client);
    if (!store) { store = { main: new NumericReply<FuelReply[]>(), awards: new NumericReply<FuelAwardsReply>() }; stores.set(client, store); }
    return store[kind];
  };
  host.__fuelRetainedRecipient = { campaign: 'c1', branch: '1', consumer: 0, continuityStartHour: 77, key: 'opaque-key',
    basis: { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9 } };
  host.__fuelDrillState = { breakdown: 'burn', follow: false, hour: 100 };
  try {
    const { default: FuelWidget } = await vite.ssrLoadModule('/ui/src/widgets/fuel/FuelWidget.tsx');
    const { contentRevision } = await vite.ssrLoadModule('/ui/src/dashboard/contentRevision.ts');
    const widget = createWidget('fuel', 'track', 'fuel-pending');
    widget.countries = { kind: 'tags', tags: ['AAA'] };
    const runtime = { tracks: [{ id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'c1', countries: [], branches: [branch] } },
      branch }], client: {}, clock: { getHour: () => 150, subscribeHour: () => () => {},
        getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } } as unknown as WidgetProps['runtime'];
    const render = (chosen = widget, scope = runtime) => {
      host.__fuelPinRequests = []; host.__fuelCharts = []; host.__fuelHistoryIdentities = [];
      return renderToStaticMarkup(createElement(FuelWidget, { widget: chosen, runtime: scope, active: true, onDiagnostics() {} }));
    };
    const originalAwards = { ...awards, awards: { ...awards.awards,
      recipients: awards.awards.recipients.map(part => ({ ...part, nameReason: 'Old native name finding' })) } };
    host.__fuelFixture = { reply, awards: originalAwards };
    assert.match(render(), /Recorded recipient A/);
    const original = host.__fuelPlot!.pickOwner;
    const plot = host.__fuelPlot!.lines.map(line => line.runs);
    const historyIdentity = host.__fuelHistoryIdentities![0];
    assert.deepEqual(host.__fuelCharts!.at(-1)!.layers[0].extent!([100, 123]), [0, 1]);
    assert.ok(original.numbers);
    contentRevision(runtime.client).observe({ game: 'synthetic-content-b', user: '', contentReady: true });
    host.__fuelFixture = {};
    const pending = render();
    assert.match(pending, /Fuel lines: 2/);
    assert.match(pending, /data-open="true"/);
    assert.doesNotMatch(pending, /Recipient names are updating/);
    assert.doesNotMatch(pending, /native reference 9007199254741000/);
    assert.doesNotMatch(pending, /Loading fuel|Recorded recipient A|Old native name finding|Synthetic land/);
    assert.equal(host.__fuelPlot!.pickOwner.numbers, original.numbers);
    assert.equal(host.__fuelPlot!.pickOwner.scope, original.scope);
    assert.deepEqual(host.__fuelPlot!.lines.map(line => line.runs), plot);
    assert.equal(host.__fuelHistoryIdentities![0], historyIdentity);
    assert.deepEqual(host.__fuelCharts!.at(-1)!.layers[0].extent!([100, 123]), [0, 1]);
    assert.deepEqual(host.__fuelPinRequests!.find(request => request.product === 'country-fuel-awards')!.params,
      host.__fuelRetainedRecipient.basis);
    const recovered: FuelReply = { ...reply, warnings: ['New content finding'], consumerNames: ['Recovered land', 'Recovered air', 'Recovered sea'],
      gainLabels: reply.gainLabels.map(item => ({ ...item, template: 'Recovered source: {NUM}', reason: 'New label finding' })) as FuelReply['gainLabels'],
      transferLabels: reply.transferLabels.map(item => ({ ...item, template: 'Recovered transfer: {NUM}', reason: null })) as FuelReply['transferLabels'] };
    const recoveredAwards = { ...awards, warnings: ['New receipt content finding'], awards: { ...awards.awards,
      recipients: awards.awards.recipients.map(part => ({ ...part, recordedName: 'Recovered recipient', nameReason: 'New native name finding' })) } };
    host.__fuelFixture = { reply: recovered, awards: recoveredAwards };
    const recovery = render();
    assert.match(recovery, /Recovered recipient/);
    assert.match(recovery, /Recovered land/);
    assert.doesNotMatch(recovery, /Recorded recipient A|Old native name finding|Recipient names are updating/);
    assert.equal(host.__fuelPlot!.pickOwner.numbers, original.numbers);
    assert.equal(host.__fuelPlot!.pickOwner.scope, original.scope);
    assert.equal(host.__fuelHistoryIdentities![0], historyIdentity);
    assert.deepEqual(host.__fuelCharts!.at(-1)!.layers[0].extent!([100, 123]), [0, 1]);
    host.__fuelFixture = { reply: recovered, awardsError: 'Synthetic recipient-name failure' };
    assert.match(render(), /Synthetic recipient-name failure/);
    host.__fuelFixture = { error: 'Synthetic content failure' };
    assert.match(render(), /Synthetic content failure/);
    host.__fuelDrillState = { breakdown: 'gain', follow: false, hour: 100 };
    const gain = { ...widget, options: { ...widget.options, show: 'gain' } };
    host.__fuelFixture = { reply: recovered };
    assert.match(render(gain), /Recovered source: 1/);
    host.__fuelFixture = {};
    const pendingGain = render(gain);
    assert.match(pendingGain, /Fuel gained \(own production\)/);
    assert.doesNotMatch(pendingGain, /Loading fuel|Recovered source|Recovered transfer|New label finding/);
    const changed: FuelReply = { ...recovered, reads: recovered.reads.map(read => ({ ...read, fuelRaw: '65536', continuityStartHour: 76 })) };
    host.__fuelFixture = { reply: changed, awards: recoveredAwards };
    render();
    assert.notEqual(host.__fuelPlot!.pickOwner.numbers, original.numbers, 'real amount/continuity changes replace numeric ownership');
    assert.equal(host.__fuelPlot!.pickOwner.numbers![0].reads[0].continuityStartHour, 76);
    host.__fuelFixture = {};
    assert.match(render({ ...widget, countries: { kind: 'tags', tags: ['BBB'] } }), /Loading fuel/);
    assert.equal(host.__fuelPlot!.pickOwner.numbers, undefined, 'a new request cannot inherit the previous countries');
    assert.match(render(widget, { ...runtime, client: {} } as WidgetProps['runtime']), /Loading fuel/);
    assert.equal(host.__fuelPlot!.pickOwner.numbers, undefined, 'a new client cannot inherit another client\'s numbers');
  } finally {
    delete host.__fuelFixture; delete host.__fuelNumberStore; delete host.__fuelDrillState; delete host.__fuelPlot;
    delete host.__fuelCharts; delete host.__fuelHistoryIdentities; delete host.__fuelPinRequests; delete host.__fuelRetainedRecipient;
    await vite.close();
  }
});

test('5,000 displayed consumers share one stack request and a fitted tooltip retaining every exact signed amount', async () => {
  const parts = Array.from({ length: 5000 }, (_, i) => ({ ...demand.parts[0], key: `native-occurrence-${i}`, ordinal: i,
    recordedName: `Recorded consumer ${i}`, amountRaw: String(i === 4999 ? -5000 : i + 1) }));
  const current = { ...demand, parts };
  const stack = { ...demandStack, parts: parts.map(part => ({ key: part.key, runs: [{ ...demandHistory.runs[0],
    amountRaw: part.amountRaw, recordedName: part.recordedName }] })) };
  const summary: FuelConsumptionReply = { ...consumption, reads: [{ ...consumption.reads[0], nativeDailyRaw: ['12492500', '0', '0'],
    knownDailyRaw: ['12492500', '0', '0'], positiveDailyRaw: ['12497500', '0', '0'] }] };
  host.__fuelFixture = { reply, consumption: summary, demand: { ...current, read: summary.reads[0] }, demandStack: stack };
  host.__fuelDrillState = { breakdown: 'consumption', follow: false, hour: 100 };
  host.__fuelPinRequests = []; host.__fuelCharts = [];
  const vite = await fuelFixture();
  try {
    const { default: FuelWidget } = await vite.ssrLoadModule('/ui/src/widgets/fuel/FuelWidget.tsx');
    const widget = createWidget('fuel', 'track', 'fuel-many-consumers');
    widget.countries = { kind: 'tags', tags: ['AAA'] };
    const runtime = { tracks: [{ id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'c1', countries: [], branches: [branch] } }, branch }],
      client: {}, clock: { getHour: () => 100, subscribeHour: () => () => {}, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } } as unknown as WidgetProps['runtime'];
    renderToStaticMarkup(createElement(FuelWidget, { widget, runtime, active: true, onDiagnostics() {} }));
    const requests = host.__fuelPinRequests!.filter(request => request.product === 'country-fuel-demand-stack');
    assert.equal(requests.length, 1);
    assert.deepEqual(requests[0].params, { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9, window: 0, keys: parts.map(part => part.key) });
    const chart = host.__fuelCharts!.find(chart => chart.label === 'Displayed individual current consumption required per day')!;
    assert.ok(Array.isArray(chart.revision));
    assert.equal(chart.revision.length, 5000, 'the viewport fit must not cap the recorded series');
    const content = chart.tooltip!(100, plotFrame(800, 400, { left: 40, right: 10, top: 10, bottom: 30 }, [100, 123], [0, 125], false), []);
    assert.ok(content);
    assert.ok(tooltipHeight(content) <= tooltipRoom(1000));
    const hidden = content.rows.find(row => row.omittedRows !== undefined)?.omittedRows;
    assert.ok(hidden && hidden > 0, 'the old unfitted tooltip mounted all 5,000 rows');
    assert.equal(content.rows.length - 1 + hidden, 5000);
    const spoken = tooltipText(content);
    const spokenRows = new Set(spoken.split('; '));
    for (const part of parts) assert.ok(spokenRows.has(`${part.recordedName} ${formatFuelPart(BigInt(part.amountRaw), 100000n)} per day`));
    assert.ok(spoken.includes('Recorded consumer 4999 −0.05 per day'));
  } finally {
    delete host.__fuelFixture; delete host.__fuelDrillState; delete host.__fuelCharts; delete host.__fuelPinRequests; delete host.__fuelPlot;
    await vite.close();
  }
});

test('the default drill routes to current individual demand and its actual selection, history, Follow and Pin callbacks', async () => {
  const vite = await fuelFixture();
  host.__fuelParts = [];
  try {
    const { default: FuelWidget } = await vite.ssrLoadModule('/ui/src/widgets/fuel/FuelWidget.tsx');
    const widget = createWidget('fuel', 'track', 'fuel-individual');
    widget.countries = { kind: 'tags', tags: ['AAA'] };
    let pinned: Parameters<WidgetProps['runtime']['addWidgetBelow']>[1] | undefined;
    const runtime = { tracks: [{ id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'c1', countries: [], branches: [branch] } }, branch }],
      client: {}, clock: { getHour: () => 110, subscribeHour: () => () => {}, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) },
      addWidgetBelow: (_id: string, value: typeof pinned) => { pinned = value; } } as unknown as WidgetProps['runtime'];
    const render = (chosen = widget) => {
      host.__fuelPinRequests = []; host.__fuelParts = []; host.__fuelCharts = [];
      return renderToStaticMarkup(createElement(FuelWidget, { widget: chosen, runtime, active: true, onDiagnostics() {} }));
    };
    render();
    host.__fuelPlot!.onPick(100, 'AAA');
    assert.equal(host.__fuelAction!.type, 'open');
    host.__fuelDrillState = reduceDrill(CLOSED, host.__fuelAction!);
    assert.equal(host.__fuelDrillState.breakdown, 'consumption', 'ordinary drill must default to the original individual consumption scope');
    const inventory = render();
    assert.match(inventory, /Current recorded division/);
    assert.match(inventory, /2\.4 per day/);
    assert.doesNotMatch(inventory, /Recorded recipient A/);
    assert.deepEqual(host.__fuelPinRequests!.find(request => request.product === 'country-fuel-demand')!.params,
      { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9 });
    assert.ok(!host.__fuelPinRequests!.some(request => request.product === 'country-fuel-awards'));
    const stackRequest = host.__fuelPinRequests!.find(request => request.product === 'country-fuel-demand-stack')!;
    assert.deepEqual(stackRequest.params, { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9, window: 0,
      keys: demand.parts.map(part => part.key) });
    const stacked = host.__fuelCharts!.find(chart => chart.label === 'Displayed individual current consumption required per day')!;
    const stackedParts = stacked.revision;
    assert.ok(Array.isArray(stackedParts));
    assert.deepEqual(stackedParts.map(part => part.id), demand.parts.map(part => part.key));
    assert.deepEqual(stacked.layers[0].extent!([100, 123]), [0, 2.4]);
    host.__fuelParts!.find(view => view.parts.some(part => part.id === '0'))!.onPick!('0');
    assert.equal(host.__fuelLevel!.level, 'family:0');
    const family = render();
    assert.match(family, /Current recorded division/);
    assert.doesNotMatch(family, /Current recorded wing/);
    assert.deepEqual(host.__fuelPinRequests!.find(request => request.product === 'country-fuel-demand-stack')!.params,
      { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9, window: 0, keys: [demand.parts[0].key] });
    const selectedLevel = host.__fuelLevel!;
    const partial: FuelConsumptionReply = { ...consumption, reads: [{ ...consumption.reads[0], nativeDailyRaw: ['240000', null, '0'],
      reasons: [null, 'Synthetic unrelated air capture failure', null] }] };
    host.__fuelFixture = { reply, consumption: partial, demand: { ...demand, read: partial.reads[0], topologyReason: 'Synthetic unrelated command failure' } };
    assert.match(render(), /Daily demand/);
    assert.equal(host.__fuelParts!.find(view => view.parts.some(part => part.id === demand.parts[0].key))!.total, 2.4,
      'an unrelated air or command failure cannot remove a complete army-family denominator');
    delete host.__fuelFixture;
    for (const stale of [{ ...selectedLevel, campaign: 'other-campaign' }, { ...selectedLevel, branch: 'other-branch' }]) {
      host.__fuelLevel = stale;
      assert.match(render(), /Not recorded at this moment/);
      assert.ok(!host.__fuelPinRequests!.some(request => request.product === 'country-fuel-demand-stack'), 'retained live scope must refuse before effects reset it');
    }
    host.__fuelLevel = selectedLevel;
    render();
    host.__fuelParts!.find(view => view.parts.some(part => part.id === demand.parts[0].key))!.onPick!(demand.parts[0].key);
    assert.equal(host.__fuelConsumption!.key, demand.parts[0].key);
    assert.doesNotMatch(render(), /native reference|Native reference/);
    const request = host.__fuelPinRequests!.find(request => request.product === 'country-fuel-demand-history')!;
    assert.deepEqual(request.params, { branchId: '1', tag: 'AAA', hour: 100, country: '7', countryInstance: 9, window: 0, key: demand.parts[0].key });
    assert.deepEqual(host.__fuelCharts!.at(-1)!.layers[0].extent!([100, 123]), [0, 2.4]);
    host.__fuelFixture = { reply, consumption, demand: { ...demand, warnings: ['Synthetic current site/topology finding'] } };
    assert.doesNotMatch(render(), /Synthetic current site\/topology finding/, 'data notes are not shown in the drill');
    host.__fuelFixture = { reply, consumption, demandError: 'Synthetic current inventory failure' };
    const failed = render();
    assert.doesNotMatch(failed, />Synthetic current inventory failure</);
    assert.match(failed, /Couldn’t load consumers/);
    assert.doesNotMatch(failed, /selected native occurrence detail is loading/);
    delete host.__fuelFixture;
    host.__fuelDrillHost!.onFollow();
    host.__fuelDrillState = reduceDrill(host.__fuelDrillState, host.__fuelAction!);
    assert.equal(host.__fuelDrillState.follow, true);
    render();
    host.__fuelCharts!.at(-1)!.onPick!(112.9);
    host.__fuelDrillState = reduceDrill(host.__fuelDrillState, host.__fuelAction!);
    assert.deepEqual(host.__fuelDrillState, { breakdown: 'consumption', follow: false, hour: 112 });
    render();
    host.__fuelDrillHost!.onPin!();
    const pinnedOptions = pinned!.options;
    assert.ok(pinnedOptions);
    assert.equal(pinnedOptions.show, 'consumption');
    assert.deepEqual(pinned!.time, { mode: 'pin', windows: { track: { firstOffset: 112, lastOffset: 112 } } });
    assert.equal(readFuelConsumption(pinnedOptions.fuelConsumption, 'c1', '1')!.basis.hour, 100);
    assert.equal(readFuelConsumptionLevel(pinnedOptions.fuelConsumptionLevel, 'c1', '1')!.level, 'family:0');
    delete host.__fuelConsumption;
    delete host.__fuelLevel;
    assert.match(render(pinned!), /Current recorded division/);
    assert.equal(pinnedOptions.fuelConsumption, fuelConsumptionToken(readFuelConsumption(pinnedOptions.fuelConsumption, 'c1', '1')!));
  } finally {
    delete host.__fuelDrillState; delete host.__fuelConsumption; delete host.__fuelAction; delete host.__fuelDrillHost;
    delete host.__fuelLevel;
    delete host.__fuelParts; delete host.__fuelCharts; delete host.__fuelPinRequests; delete host.__fuelPlot;
    delete host.__fuelFixture;
    await vite.close();
  }
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../../charts/frame.ts';
import { CLOSED } from '../../charts/drill/drillState.ts';
import type { DrillState } from '../../charts/drill/drillState';
import type { DrillPart } from '../../charts/drill/parts';
import type { TimeChartProps } from '../../charts/TimeChart';
import { createWidget } from '../../dashboard/model.ts';
import type { WidgetSpec } from '../../dashboard/model';
import type { WidgetProps } from '../../dashboard/definitions';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { contentKey, resolveOptions, setToken } from '../../dashboard/options.ts';
import { PlaybackController } from '../../dashboard/playback.ts';
import { object } from '../../dashboard/validation.ts';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { BranchMetadata, CountryGroupReply, EquipmentStockpileReply, HourSpan, StockpileArtworkReply, StockpileCatalogueReply, StockpileChoicePeak,
  StockpileHolder } from '../../generated/dashboard';
import type { TooltipContent } from '../../charts/tooltip';
import { clipToSpans } from '../../charts/series.ts';
import { definition, OPTIONS } from './definition.ts';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import { datedCountryToken } from '../../dashboard/datedCountry.ts';
import { balanceAt, shareRuns, stockAt, stockGroups, stockRuns, variantKey } from './stockpileData.ts';
import { stockpileGroupKey } from './stockpileCatalogue.ts';

registerWidgets([definition]);
const span = { firstHour: 100, lastHour: 200 };
const raw = (amountRaw: string) => [{ ...span, amountRaw }];
const reply: EquipmentStockpileReply = {
  branchId: '1', tag: 'AAA', campaignKey: 'campaign-a', decimals: 5, warnings: [],
  reads: [100], logisticsReads: [100], lineReads: [100], stockHours: [span], balanceHours: [span], lineHours: [span], aiHours: [],
  needBoundRaw: '9223372036854775807',
  types: [{ key: 'mod/rifles', name: 'Authored rifles', domain: 'land', needNote: null },
    { key: 'mod/trucks', name: 'Authored trucks', domain: 'land', needNote: null }],
  variants: [{ id: '9007199254741001', token: 'mod_rifle_a', archetype: 'mod/rifles',
    names: [{ firstHour: 100, known: true, name: 'Recorded rifles', design: 'design-a' }], stock: raw('1200000'), daily: raw('99900000'), dailyIc: raw('4500000'), unitCostLow: raw('55000'), unitCostHigh: raw('65000') },
    { id: '9007199254741002', token: 'mod_truck_a', archetype: 'mod/trucks', names: [], stock: raw('800000'), daily: [], dailyIc: [], unitCostLow: [], unitCostHigh: [] }],
  logistics: [{ key: 'mod/rifles', available: raw('2000000'), balance: raw('500000'), stock: raw('99900000') },
    { key: 'mod/trucks', available: raw('800000'), balance: raw('-200000'), stock: raw('800000') }],
};
const branch: BranchMetadata = { id: '1', sessionId: '1', firstHour: 0, lastHour: 200,
  parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] };
const unusedTransport = async () => { throw new Error('The stockpile fixture must replace this transport.'); };
const fixtureClient = (): DashboardClient => ({ available: true, openSource: unusedTransport, releaseSource: unusedTransport,
  product: unusedTransport, setMapViews: unusedTransport, onMapStates: unusedTransport });
interface PickOwner { scope: string; client: object }

/** The catalogue preparation answers for these holders' replies, each within its hours, from the
 * widget's own at-hour reading of them. */
function catalogueOf(replies: readonly EquipmentStockpileReply[], holders: readonly StockpileHolder[]): StockpileCatalogueReply {
  const within = (spans: readonly HourSpan[], at: number) => holders[at]?.within ? clipToSpans(spans.map(span => ({ ...span, value: 0, breakBefore: false })),
    holders[at].within!).map(({ firstHour, lastHour }) => ({ firstHour, lastHour })) : [...spans];
  const spans = (pick: (reply: EquipmentStockpileReply) => readonly HourSpan[]) =>
    replies.flatMap((reply, at) => within(pick(reply), at)).sort((a, b) => a.firstHour - b.firstHour);
  const hours = (pick: (reply: EquipmentStockpileReply) => readonly number[]) => [...new Set(replies.flatMap((reply, at) => pick(reply)
    .filter(hour => !holders[at]?.within || holders[at].within!.some(span => span.firstHour <= hour && hour <= span.lastHour))))].sort((a, b) => a - b);
  const variants = new Map<string, StockpileCatalogueReply['variants'][number]>();
  const logistics = new Map<string, boolean>();
  const types = new Map<string, StockpileCatalogueReply['types'][number]>();
  const choices: Record<string, StockpileChoicePeak> = {};
  for (const reply of replies) {
    for (const variant of reply.variants) if (!variants.has(variant.id)) variants.set(variant.id, { id: variant.id, token: variant.token,
      archetype: variant.archetype, names: variant.names, designs: [...new Set(variant.names.flatMap(name => name.design ?? []))] });
    for (const item of reply.logistics) logistics.set(item.key, (logistics.get(item.key) ?? false)
      || [...item.balance, ...item.stock, ...item.available].some(run => run.amountRaw !== '0'));
    for (const type of reply.types) if (!types.has(type.key)) types.set(type.key, type);
    for (const grouping of ['type', 'domain', 'variant'] as const) for (const group of stockGroups(reply, grouping, Infinity)) {
      const stock = stockRuns(reply, group), share = shareRuns(reply, group);
      const peak = [...reply.stockHours, ...reply.balanceHours].reduce((most, span) => {
        const held = stockAt(reply, group, span.firstHour) ?? 0n, need = balanceAt(reply, group, span.firstHour)?.need ?? 0n;
        return [most, held, need].reduce((a, b) => a > b ? a : b);
      }, 0n);
      const kept = choices[stockpileGroupKey(group)];
      choices[stockpileGroupKey(group)] = { raw: String(kept && BigInt(kept.raw) > peak ? BigInt(kept.raw) : peak),
        stockDrawable: !!kept?.stockDrawable || stock.length > 0, shareDrawable: !!kept?.shareDrawable || share.length > 0 };
    }
  }
  return { branchId: replies[0].branchId, tags: holders.map(holder => holder.tag), campaignKey: replies[0].campaignKey, decimals: replies[0].decimals,
    types: [...types.values()], variants: [...variants.values()], logistics: [...logistics].map(([key, nonzero]) => ({ key, nonzero })),
    reads: hours(reply => reply.reads), lineReads: hours(reply => reply.lineReads), logisticsReads: hours(reply => reply.logisticsReads),
    stockHours: spans(reply => reply.stockHours), lineHours: spans(reply => reply.lineHours), balanceHours: spans(reply => reply.balanceHours),
    aiHours: spans(reply => reply.aiHours), needBoundRaw: replies[0].needBoundRaw, choices, warnings: [...new Set(replies.flatMap(reply => reply.warnings))] };
}
interface ChoiceOwner { pickOwner: object; chosenScope: string; extent: readonly number[] }
const host = globalThis as typeof globalThis & {
  __stockCountryPick?: { owner: object; selection: DatedCountrySelection } | null;
  __stockCatalogue?: CountryGroupReply; __stockReplies?: EquipmentStockpileReply[];
  __stockFollow?: () => void;
  __stockChip?: { props: { hour: number | null; label: { name: string } | null } };
  __stockCatalogueNumbers?: (client: object) => NumericReply<StockpileCatalogueReply>;
  __stockReply?: EquipmentStockpileReply; __stockRequests?: ProductRequest[]; __stockWithin?: HourSpan[];
  __stockHourNumbers?: (client: object) => NumericReply<EquipmentStockpileReply>;
  __stockCatalogueOf?: typeof catalogueOf;
  __stockOwner?: (next: PickOwner) => PickOwner; __stockChoiceOwner?: (next: ChoiceOwner) => ChoiceOwner;
  __stockDrill?: DrillState; __stockDrillOwner?: object | null;
  __stockSelection?: { owner: object; key: string; mode: string } | null;
  __stockChoices?: { owner: object; hour: number; keys: readonly string[] } | null;
  __stockRowPick?: (key: string, callback: () => void) => () => void;
  __stockCharts?: TimeChartProps[]; __stockParts?: readonly DrillPart[]; __stockHour?: number;
  __stockHost?: { open: boolean; mode: string; title: string; returnFocus?: string; onPin?: () => void; onFollow: () => void };
  __stockArtwork?: StockpileArtworkReply; __stockTooltip?: (key: string) => TooltipContent | null;
};

/** Render the actual country widget, acquisition hook, picker, part calculation and
 * stack. Only transport/DOM leaves are replaced. SSR seeds committed state and memo
 * ownership; it cannot certify browser portals, effects or animation. */
async function fixture() {
  const owners = new WeakMap<object, PickOwner>();
  let choice: ChoiceOwner | undefined;
  // Each client keeps its held numbers across renders, as its mounted hooks would.
  const stores = <T,>() => {
    const kept = new WeakMap<object, NumericReply<T>[]>();
    let calls = 0;
    return (client: object) => {
      const list = kept.get(client) ?? []; kept.set(client, list);
      const at = calls++ % 2;
      return (list[at] ??= new NumericReply<T>());
    };
  };
  host.__stockCatalogueNumbers = stores<StockpileCatalogueReply>();
  host.__stockHourNumbers = stores<EquipmentStockpileReply>();
  host.__stockCatalogueOf = catalogueOf;
  host.__stockOwner = next => {
    const previous = owners.get(next.client);
    if (previous && previous.scope === next.scope) return previous;
    owners.set(next.client, next); return next;
  };
  host.__stockChoiceOwner = next => {
    if (choice && choice.pickOwner === next.pickOwner && choice.chosenScope === next.chosenScope
      && choice.extent.every((value, index) => value === next.extent[index])) return choice;
    choice = next; return next;
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'stockpile-drill-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts')) return `function answer(request) {
          const replies = globalThis.__stockReplies, reply = globalThis.__stockReply;
          const of = tag => replies?.find(item => item.tag === tag) ?? reply;
          if (request.product === 'equipment-stockpile-catalogue') {
            const held = request.params.holders.map(holder => of(holder.tag));
            return held.every(Boolean) ? globalThis.__stockCatalogueOf(held, request.params.holders) : undefined;
          }
          return of(request.params.tag);
        }
        export function useProduct(_client, request, active, validate) {
          if (!request || !active) return { retry() {} };
          globalThis.__stockRequests?.push(request);
          const data = answer(request);
          return { data: data ? validate(data) : undefined, retry() {} };
        }
        export async function loadProduct() { throw new Error('The stockpile fixture reads its chart lines at once.'); }
        // A followed read answers the hour it was asked for at once.
        export function useFollowedProduct(client, request, moving, active, validate) {
          const product = useProduct(client, request, active, validate);
          return { ...product, moment: product.data === undefined ? undefined : JSON.stringify(moving.map(name => request.params[name])) };
        }
        export function useProducts(_client, requests, active, validate) {
          if (active) globalThis.__stockRequests?.push(...requests);
          const replies = requests.map(request => globalThis.__stockReplies?.find(reply => reply.tag === request.params.tag) ?? globalThis.__stockReply);
          return { data: replies.length && replies.every(Boolean) ? replies.map(validate) : undefined, retry() {} };
        }
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`;
        if (path.endsWith('/dashboard/viewRead.ts')) return `import { shareRuns, stockGroups, stockRuns } from '../widgets/equipment-stockpile/stockpileData.ts';
          // Chart lines are read at once, from the widget's own reading of the hour's reply.
          export function useViewRead(reads, view) {
            const reply = globalThis.__stockReply;
            if (reads == null || !view || !reply) return { data: undefined, retry() {} };
            const lines = JSON.parse(JSON.parse(reads)[0])[5];
            const groups = ['type', 'domain', 'variant'].flatMap(grouping => stockGroups(reply, grouping, Infinity));
            const key = group => JSON.stringify([group.kind, group.kind === 'variant' ? group.variants[0]?.id : group.key]);
            return { data: new Map(lines.map(line => {
              const group = groups.find(item => key(item) === line.group);
              const runs = !group || line.measure === 'need' ? [] : line.measure === 'share' ? shareRuns(reply, group) : stockRuns(reply, group);
              // The TimeChart leaf never reports a viewport; use the picking frame's hours and width.
              return [line.key, { from: 0, to: 200, width: 350, buckets: [], runs }];
            })), retry() {} };
          }`;
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return `export const useCountryGroups = (_runtime, track) => ({
          data: globalThis.__stockCatalogue ?? { branchId: track?.branch.id, warnings: [], humans: [], factions: [], wars: [], warStarts: [], capitulations: [],
            identityChanges: [], unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [
              { tag: 'AAA', identities: [{ firstHour: track?.branch.firstHour, lastHour: track?.branch.lastHour, slot: 7, countryId: 9001 }] }] }, retry() {} });`;
        if (path.endsWith('/dashboard/countryFlagsAt.ts')) return `export const useCountryLabelsAt = () => ({
          labelAt: hour => tag => ({ name: tag + ' at ' + hour, known: true, flag: 'data:flag-' + tag + '@' + hour }) });`;
        if (path.endsWith('/dashboard/CountryComparisons.tsx')) return `import { createElement } from 'react';
          export default ({ component, ...props }) => createElement(component, { ...props, within: globalThis.__stockWithin });`;
        if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = clock => clock.getHour();';
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => ({ accent: '#123', muted: '#888',
          categories: ['#123', '#456', '#789'], motion: 100, reducedMotion: true }); export const useViewportHeight = () => 1000;`;
        if (path.endsWith('/charts/useRowMotion.ts')) return `export const useRowMotion = rows => rows.map((row, slot) => ({ ...row, slot, phase: 'stable' }));
          export const movingRowClass = () => 'moving-row';`;
        if (path.endsWith('/charts/useRowTooltips.ts')) return `export const useRowTooltips = props => {
          globalThis.__stockTooltip = props.content; return { rowProps: () => ({ tabIndex: 0 }), descriptionId: key => key }; };`;
        if (path.endsWith('/equipment-stockpile/useStockpileArtwork.ts')) return 'export const useStockpileArtwork = () => ({ data: globalThis.__stockArtwork });';
        if (path.endsWith('/charts/TimeChart.tsx')) return 'export default props => { globalThis.__stockCharts?.push(props); return null; };';
        // The table's rows register their activation and tooltip by key, as the kit's would.
        if (path.endsWith('/charts/table/Table.tsx')) return `import { createElement } from 'react';
          export default props => {
            for (const row of props.rows) globalThis.__stockRowPick?.(props.rowKey(row), () => props.onRowClick?.(row));
            globalThis.__stockTooltip = key => { const row = props.rows.find(item => props.rowKey(item) === key); return row && props.tooltip ? props.tooltip(row) : null; };
            return createElement('table', null, createElement('tbody', null, props.rows.map(row => createElement('tr', { key: props.rowKey(row) },
              props.columns.map(column => createElement('td', { key: column.key }, column.render ? column.render(row) : String(column.value(row))))))));
          };`;
        if (path.endsWith('/charts/drill/DrillParts.tsx')) return source.replace('const rows = useMemo(', 'globalThis.__stockParts = parts; const rows = useMemo(');
        if (path.endsWith('/charts/drill/DrillHost.tsx')) return `export default props => {
          globalThis.__stockHost = props; return props.open ? props.children : props.main;
        };`;
        if (path.endsWith('/equipment-stockpile/useEquipmentStockpile.ts')) return source.replaceAll('new NumericReply<StockpileCatalogueReply>()',
          'globalThis.__stockCatalogueNumbers(client)').replaceAll('new NumericReply<EquipmentStockpileReply>()', 'globalThis.__stockHourNumbers(client)');
        if (path.endsWith('/equipment-stockpile/StockpileDrill.tsx')) return source.replace('const tokens = useChartTokens()',
          'globalThis.__stockHour = hour; const tokens = useChartTokens()');
        if (path.endsWith('/equipment-stockpile/EquipmentStockpileWidget.tsx')) return source
          .replace('({ scope, client: runtime.client })', 'globalThis.__stockOwner({ scope, client: runtime.client })')
          .replace('({ pickOwner, chosenScope, extent })', 'globalThis.__stockChoiceOwner({ pickOwner, chosenScope, extent })')
          .replace('const [drill, dispatch] = useReducer(reduceDrill, CLOSED);', `const [drill] = useReducer(reduceDrill, globalThis.__stockDrill ?? CLOSED);
            const dispatch = action => { globalThis.__stockDrill = reduceDrill(globalThis.__stockDrill ?? CLOSED, action); };`)
          .replace('const [drillOwner, setDrillOwner] = useState<object | null>(null);', `const [drillOwner] = useState(globalThis.__stockDrillOwner ?? null);
            const setDrillOwner = owner => { globalThis.__stockDrillOwner = owner; };`)
          .replace('const [selection, setSelection] = useState<{ owner: object; key: string; mode: StockpileMode } | null>(null);',
            `const [selection] = useState(globalThis.__stockSelection ?? null); const setSelection = value => { globalThis.__stockSelection = value; };`)
          .replace('const [selection, setSelection] = useState<{ owner: object; hour: number; keys: readonly string[] } | null>(null);',
            `const [selection] = useState(globalThis.__stockChoices ?? null); const setSelection = value => { globalThis.__stockChoices = value; };`)
          .replace('const [countryPick, setCountryPick] = useState<{ owner: object; selection: DatedCountrySelection } | null>(null);',
            `const [countryPick] = useState(globalThis.__stockCountryPick ?? null); const setCountryPick = value => { globalThis.__stockCountryPick = value; };`)
          .replace('onClick={() => act(', 'onClick={globalThis.__stockFollow = () => act(')
          .replace('createPortal(', '((chip) => { globalThis.__stockChip = chip; return null; })(')
          .replace('onClick={pick}', 'onClick={globalThis.__stockRowPick(row.key, pick)}');
      },
    }] });
  const metadata = await vite.ssrLoadModule('/ui/src/widgets/equipment-stockpile/definition.ts');
  const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
  registry.registerWidgets([metadata.definition]);
  const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/equipment-stockpile/EquipmentStockpileWidget.tsx');
  const rows = new Map<string, () => void>(), pins: WidgetSpec[] = [];
  host.__stockRowPick = (key, callback) => { rows.set(key, callback); return callback; };
  const widget = createWidget('equipment-stockpile', 'track', 'stockpile-source');
  widget.countries = { kind: 'tags', tags: ['AAA'] };
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: 200 }); clock.seek(150); clock.setSpeed(24);
  const runtime = { tracks: [{ id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'campaign-a' } }, branch }],
    client: fixtureClient(), clock,
    addWidgetBelow: (_id: string, pinned: WidgetSpec) => pins.push(pinned) } as unknown as WidgetProps['runtime'];
  host.__stockReply = reply;
  const render = (chosen = widget, scope = runtime, active = true) => {
    host.__stockRequests = []; host.__stockCharts = []; rows.clear();
    return renderToStaticMarkup(createElement(Widget, { widget: chosen, runtime: scope, active, countrySlot: {} as HTMLElement, onDiagnostics() {}, setOption() {} }));
  };
  return { widget, runtime, rows, pins, render, async close() {
    for (const key of ['__stockCountryPick', '__stockCatalogue', '__stockReplies', '__stockChip', '__stockFollow', '__stockCatalogueNumbers', '__stockReply', '__stockRequests', '__stockWithin', '__stockHourNumbers', '__stockCatalogueOf', '__stockOwner', '__stockChoiceOwner', '__stockDrill',
      '__stockDrillOwner', '__stockSelection', '__stockChoices', '__stockRowPick', '__stockCharts', '__stockParts', '__stockHour', '__stockHost', '__stockArtwork', '__stockTooltip'] as const) delete host[key];
    await vite.close();
  } };
}

test('stockpile artwork slots use a fallback and best-design names appear only on hover', async () => {
  const view = await fixture();
  try {
    host.__stockArtwork = { branchId: '1', tag: 'AAA', hour: 100, warnings: [], entries: [
      { key: 'type:mod/rifles', variant: '9007199254741001', name: 'Best rifle design', icon: null, reason: 'Native picture refused' },
    ] };
    const html = view.render();
    assert.equal((html.match(/class="art-fallback"/g) ?? []).length, 2);
    // The best design's name is the type's picture, named on hover; rows name the models held.
    assert.doesNotMatch(html, /Best rifle design|Native picture refused/);
    assert.match(html, /<small>Recorded rifles · 12 in stock<\/small>/);
    const tooltip = host.__stockTooltip!('mod/rifles')!;
    assert.match(tooltip.note ?? '', /Best rifle design/);
    assert.doesNotMatch(tooltip.note ?? '', /Picture:|read |Native picture refused/);
  } finally { await view.close(); }
});

test('a reloaded stockpile pin dates its body/header through local picks, Follow, rename and tag reuse', async () => {
  const view = await fixture();
  try {
    host.__stockCatalogue = { branchId: '1', warnings: [], humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
      unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [
        { tag: 'AAA', identities: [{ firstHour: 0, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 200, slot: 2, countryId: 20 }] },
        { tag: 'Y', identities: [{ firstHour: 110, lastHour: 114, slot: 1, countryId: 10 }, { firstHour: 117, lastHour: 200, slot: 3, countryId: 30 }] },
      ] };
    host.__stockReplies = [reply, { ...reply, tag: 'Y', variants: reply.variants.map(variant => ({ ...variant, stock: raw('1300000') })) }];
    const pinned = { ...view.widget, time: { mode: 'pin', windows: { track: { firstOffset: 105, lastOffset: 105 } } }, options: {
      show: 'stockpile', partMode: 'stock', partGroup: 'mod:2f:rifles', stockpileCountry: datedCountryToken({ campaign: 'campaign-a', branch: '1', tag: 'AAA', originOffset: 105 }),
    } } as WidgetSpec;
    view.render(pinned);
    assert.equal(host.__stockHour, 105);
    assert.equal(host.__stockChip!.props.label!.name, 'AAA at 105');
    assert.match(view.render({ ...pinned, time: { mode: 'pin', windows: {} } }), /Choose another time in widget settings/);
    assert.equal(host.__stockChip!.props.label, null);
    assert.equal(host.__stockChip!.props.hour, null);
    assert.equal(host.__stockCharts!.length, 0, 'a missing saved date cannot display the branch end');
    view.render(pinned);
    host.__stockCharts!.at(-1)!.onPick!(112); view.render(pinned);
    assert.equal(host.__stockChip!.props.label!.name, 'Y at 112');
    assert.equal(host.__stockParts![0].raw, 1300000n);
    host.__stockReply = undefined; view.render(pinned, { ...view.runtime, client: fixtureClient() });
    assert.equal(host.__stockHour, 105, 'the alias body opens without the literal-tag overview');
    host.__stockReply = reply; view.render(pinned);
    view.runtime.clock.seek(113); host.__stockFollow!(); view.render(pinned);
    assert.equal(host.__stockHour, 113);
    assert.equal(host.__stockChip!.props.label!.name, 'Y at 113');
    view.runtime.clock.seek(116); view.render(pinned);
    assert.equal(host.__stockChip!.props.label, null);
    assert.equal(host.__stockParts![0].raw, null);
    view.runtime.clock.seek(119); view.render(pinned);
    assert.equal(host.__stockChip!.props.label, null, 'neither replacement supplies the dated header');
    const invalid = { ...pinned, options: { ...pinned.options, stockpileCountry: datedCountryToken({ campaign: 'other', branch: '1', tag: 'AAA', originOffset: 105 }) } };
    assert.match(view.render(invalid), /Choose a country again/);
    assert.equal(host.__stockChip!.props.label, null);
  } finally { await view.close(); }
});

test('a breakdown pinned by variant, when variants could be grouped, still opens on its variant and design', async () => {
  const view = await fixture();
  try {
    const pinned = { ...view.widget, time: { mode: 'pin', windows: { track: { firstOffset: 150, lastOffset: 150 } } }, options: {
      show: 'stockpile', grouping: 'variant', partMode: 'stock', partGroup: setToken(`${variantKey(reply, reply.variants[0])}#design-a`),
      stockpileCountry: datedCountryToken({ campaign: 'campaign-a', branch: '1', tag: 'AAA', originOffset: 150 }),
    } } as WidgetSpec;
    assert.doesNotMatch(view.render(pinned), /not defined by this campaign/);
    assert.deepEqual(host.__stockParts!.map(part => [part.label, part.raw]), [['Recorded rifles', 1200000n]]);
    // Another design under the same handle is not drawn as the saved one.
    const other = { ...pinned, options: { ...pinned.options, partGroup: setToken(`${variantKey(reply, reply.variants[0])}#design-b`) } };
    assert.match(view.render(other), /not defined by this campaign/);
  } finally { await view.close(); }
});

test('an actual row opens inside, history holds its picked day, Follow uses playback and Pin keeps country/type/time', async () => {
  const view = await fixture();
  try {
    assert.match(view.render(), /Authored rifles/);
    assert.equal(host.__stockHost!.mode, 'inline');
    view.rows.get('mod/rifles')!(); view.render();
    assert.equal(host.__stockHost!.open, true);
    assert.equal(host.__stockHost!.returnFocus, '.stockpile-table-rows tr');
    // The breakdown lists the type's models in stock, in whole units.
    assert.deepEqual(host.__stockParts!.map(part => [part.label, part.raw, part.text]), [['Recorded rifles', 1200000n, '12']]);
    // A new widget follows playback, so the row opens following it.
    assert.deepEqual(host.__stockDrill, { breakdown: 'stockpile', follow: true, hour: null });
    assert.equal(host.__stockHour, 150);
    host.__stockCharts!.at(-1)!.onPick!(110); view.render();
    assert.deepEqual(host.__stockDrill, { breakdown: 'stockpile', follow: false, hour: 110 });
    assert.equal(host.__stockHour, 110);
    host.__stockHost!.onPin!();
    const pinned = view.pins[0], options = resolveOptions(OPTIONS, pinned.options);
    assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['AAA'] });
    assert.equal(contentKey(String(options.partGroup)), 'mod/rifles');
    assert.equal(pinned.options?.partMode, 'balance');
    assert.deepEqual(pinned.time, { mode: 'pin', windows: { track: { firstOffset: 110, lastOffset: 110 } } });
    host.__stockHost!.onFollow(); view.render();
    assert.equal(host.__stockHour, 150);
    view.render({ ...view.widget, options: { drill: 'popout' } });
    assert.equal(host.__stockHost!.mode, 'popout');
    // The game's logistics equation: 20 available − 15 needed = a surplus of 5.
    assert.match(view.render(pinned), /Available <b>20<\/b> − needed <b>15<\/b> = <b class="is-positive">surplus \+5<\/b>/);
    // Per model: the game's output a day, the IC its lines put in a day, and their lowest to highest cost per unit.
    assert.match(view.render(pinned), /<span role="cell">Recorded rifles<\/span><b role="cell">999<\/b><b role="cell">45<\/b><b role="cell">0\.55–0\.65<\/b>/);
    const catalogueHolders = new Set<string>();
    for (const request of host.__stockRequests!) {
      assert.ok(object(request.params));
      if (request.product === 'equipment-stockpile-at') assert.equal(request.params.tag, 'AAA');
      else {
        assert.equal(request.product, 'equipment-stockpile-catalogue');
        catalogueHolders.add(JSON.stringify(request.params.holders));
      }
    }
    assert.deepEqual(catalogueHolders, new Set(['[{"tag":"AAA"}]', '[{"tag":"AAA","within":[{"firstHour":0,"lastHour":200}]}]']));
  } finally { await view.close(); }
});

test('an aligned campaign follows its native hour and pins the type with its models named as recorded then', async () => {
  const view = await fixture();
  try {
    const widget = view.widget;
    const dated = { ...reply, variants: [{ ...reply.variants[0], names: [
      { firstHour: 100, known: true, name: 'Original rifles', design: 'original-design' },
      { firstHour: 140, known: true, name: 'Aligned rifles', design: 'aligned-design' },
      { firstHour: 180, known: true, name: 'Later rifles', design: 'later-design' },
    ] }, reply.variants[1]] };
    host.__stockReply = dated;
    const clock = Object.assign(new PlaybackController(), { getHour: () => 50.75,
      getSnapshot: () => ({ ...view.runtime.clock.getSnapshot(), playing: true }) });
    const runtime = { ...view.runtime, tracks: [{ ...view.runtime.tracks[0], axisOffset: 100,
      branch: { ...branch, firstHour: 100 } }], clock };
    view.render(widget, runtime);
    const row = view.rows.get('mod/rifles')!;
    row(); view.render(widget, runtime);
    assert.equal(host.__stockHost!.open, true);
    assert.equal(host.__stockDrill!.follow, true);
    assert.equal(host.__stockHour, 150);
    assert.equal(host.__stockHost!.title, 'Authored rifles');
    assert.deepEqual(host.__stockParts!.map(part => [part.label, part.raw]), [['Aligned rifles', 1200000n]]);
    host.__stockHost!.onPin!();
    const following = view.pins[0];
    assert.equal(following.title, `${widget.title}: Authored rifles`);
    assert.equal(contentKey(String(resolveOptions(OPTIONS, following.options).partGroup)), 'mod/rifles');
    assert.deepEqual(following.time, { mode: 'follow' });
    host.__stockCharts!.at(-1)!.onPick!(110); view.render(widget, runtime);
    assert.equal(host.__stockHour, 110);
    assert.deepEqual(host.__stockParts!.map(part => part.label), ['Original rifles']);
    host.__stockHost!.onPin!();
    assert.deepEqual(view.pins[1].time, { mode: 'pin', windows: { track: { firstOffset: 10, lastOffset: 10 } } });
    host.__stockHost!.onFollow(); view.render(widget, runtime);
    assert.equal(host.__stockHour, 150);
    assert.deepEqual(host.__stockParts!.map(part => part.label), ['Aligned rifles']);
    const replacement = { ...runtime, client: fixtureClient() };
    view.render(widget, replacement);
    assert.equal(host.__stockHost!.open, false);
    row(); view.render(widget, replacement);
    assert.equal(host.__stockHost!.open, false);
  } finally { await view.close(); }
});

test('actual time-chart pick uses the nearest drawn equipment and leaves overlapping lines for a chooser', async () => {
  const view = await fixture();
  try {
    const widget = { ...view.widget, options: { view: 'time' } };
    view.render(widget);
    const frame = plotFrame(400, 300, { left: 30, right: 20, top: 20, bottom: 30 }, [0, 200], [-100, 150], false);
    host.__stockCharts![0].onPick!(110, { frame, point: { x: frame.toX(110), y: frame.toY(100 / 3) }, until: 200 });
    assert.equal(host.__stockSelection!.key, 'mod/rifles');
    view.render(widget);
    assert.equal(host.__stockHost!.title, 'Authored rifles');
    host.__stockDrill = CLOSED; host.__stockSelection = null;
    host.__stockReply = { ...reply, logistics: [{ ...reply.logistics[0] }, { ...reply.logistics[0], key: 'mod/trucks' }] };
    view.render(widget);
    host.__stockCharts![0].onPick!(110, { frame, point: { x: frame.toX(110), y: frame.toY(100 / 3) }, until: 200 });
    assert.deepEqual(host.__stockChoices!.keys, ['mod/rifles', 'mod/trucks']);
    assert.equal(host.__stockSelection, null);
    assert.match(view.render(widget), /Choose drawn equipment/);
    assert.equal(host.__stockHost!.open, false);
    host.__stockCharts![0].onPick!(110, { frame, point: { x: frame.toX(110), y: frame.toY(100 / 3) }, until: 109 });
    assert.equal(host.__stockChoices, null);
  } finally { await view.close(); }
});

test('actual request/render ownership retains held numeric parts through content refresh and rejects a replacement client', async () => {
  const view = await fixture();
  try {
    host.__stockWithin = [span];
    view.render(); view.rows.get('mod/rifles')!(); view.render();
    const owner = host.__stockDrillOwner, parts = host.__stockParts!.map(part => part.raw);
    host.__stockReply = undefined;
    host.__stockWithin = [{ ...span }];
    const pending = view.render();
    assert.doesNotMatch(pending, /Authored rifles/);
    assert.equal(host.__stockHost!.open, true);
    assert.equal(host.__stockDrillOwner, owner);
    assert.deepEqual(host.__stockParts!.map(part => part.raw), parts);
    host.__stockReply = { ...reply, types: reply.types.map(type => ({ ...type, name: 'New content label' })) };
    view.render();
    assert.equal(host.__stockHost!.title, 'New content label');
    assert.equal(host.__stockDrillOwner, owner);
    view.render(view.widget, view.runtime, false);
    assert.deepEqual(host.__stockRequests, []);
    host.__stockReply = undefined;
    assert.match(view.render(view.widget, { ...view.runtime, client: fixtureClient() }), /Loading the stockpile/);
  } finally { await view.close(); }
});

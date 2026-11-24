import { gatedTime } from '../../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { stackEdges } from '../../charts/stack.ts';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { registerWidgets } from '../../dashboard/definitions.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { parseStoredOptions, resolveOptions } from '../../dashboard/options.ts';
import { definition, OPTIONS } from './definition.ts';
import { buildDiagram, decodeTrade, flowCountryAt, flowValue } from './tradeData.ts';
import { contributionAmounts, contributionCountryTag, contributionExtent, contributionHistory, linkSelection, nodeSelections,
  contributionMeasureExtent, contributionMeasureModel, forEachContributionMeasureSegment,
  observedTradeSpans, pinContributions, readSelection, selectedContributors, selectionToken, tradeContext } from './tradeDrill.ts';
import { regionHour, type AxisHour } from '../../dashboard/regionTime.ts';
import { drillAction } from '../../charts/drill/drillState.ts';
import { createElement } from 'react';
import type { ComponentProps } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type TradeDrill from './TradeDrillView';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { TradeFlowsReply } from '../../generated/dashboard';

const identity = (slot: number) => JSON.stringify([slot, slot + 1]);
const CONTEXT = { campaignKey: 'campaign-one', branchId: 'b' };

test('a single-country trade pin dates its native cohort header through rename, gaps and tag reuse', () => {
  const index = countryIdentityIndex([
    { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 160, slot: 2, countryId: 20 }] },
    { tag: 'Y', identities: [{ firstHour: 120, lastHour: 140, slot: 1, countryId: 10 }, { firstHour: 141, lastHour: 160, slot: 3, countryId: 30 }] },
  ]);
  const selection = { kind: 'node', side: 'source', holders: ['[1,10]'] } as const;
  assert.equal(contributionCountryTag(index, 'X', selection, 105), 'X');
  assert.equal(contributionCountryTag(index, 'X', selection, 125), 'Y');
  assert.equal(contributionCountryTag(index, 'X', selection, 115), null);
  assert.equal(contributionCountryTag(index, 'X', selection, 150), null);
  assert.equal(contributionCountryTag(index, 'X', { kind: 'all' }, 125), null, 'multiple native holders cannot become one header');
});

function reply(count: number): TradeFlowsReply {
  const countries = Array.from({ length: count + 1 }, (_, slot) => `Country ${String(slot).padStart(5, '0')}`);
  return { branchId: 'b', decimals: 0, countries,
    countryIdentities: countries.map((tag, slot) => ({ tag, runs: [{ firstHour: 0, lastHour: 100, slot, countryId: slot + 1 }] })),
    resources: [{ id: 7, token: 'mod/resource', ordinal: 1, name: null, icon: null }], firstHour: 0, lastHour: 100,
    known: [{ firstHour: 0, lastHour: 71 }], partial: [], warnings: [],
    flows: Array.from({ length: count }, (_, seller) => ({ seller, buyer: count, sellerIdentity: identity(seller), buyerIdentity: identity(count),
      sellerTags: [seller], buyerTags: [count], resource: 7, firstHours: [0], lastHours: [47], deliveredRaw: [String(count - seller)],
      factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] })) };
}


test('contributor totals add the recorded hours before a gap, which hides only the current amount', async () => {
  const state = { chart: undefined as TimeChartProps | undefined };
  const host = globalThis as typeof globalThis & { __tradeDrillPresentation?: typeof state }; host.__tradeDrillPresentation = state;
  const replacements: Record<string, string> = {
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryLooks.ts': 'export const lookHour = (_branch, hour) => ({ hour });',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/drill/DrillParts.tsx': 'export default props => `Total ${props.totalRow.value}`;',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__tradeDrillPresentation.chart = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'trade-drill-cumulative', enforce: 'pre', transform(_code, id) {
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Component } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeDrillView.tsx');
    const trade = decodeTrade({ ...reply(2), known: [{ firstHour: 0, lastHour: 20 }, { firstHour: 31, lastHour: 71 }] });
    const track = { id: 'main', source: { id: 'source', revision: '1' }, branch: { id: 'b', firstHour: 0, lastHour: 71, gaps: [] } } as unknown as BoundTrack;
    const props = (measure: string) => ({ flows: trade.flows, trade, track, runtime: { client: {}, clock: { getSnapshot: () => ({ hoursPerSecond: 0 }) } },
      measure, from: 0, hour: 25, live: null, extent: [0, 71], known: trade.reply.known, colours: new Map(),
      tokens: { muted: '#888888' }, active: true, onPick() {}, onContextMenu() {} }) as unknown as ComponentProps<typeof TradeDrill>;
    const tip = () => state.chart!.tooltip!(25, undefined as never, [])!;
    assert.match(renderToStaticMarkup(createElement(Component, props('total'))), /Total (?!Not recorded)/);
    assert.notEqual(tip().rows[0]?.value, 'Not recorded');
    renderToStaticMarkup(createElement(Component, props('current')));
    assert.equal(tip().rows[0]?.value, 'Not recorded', 'only the current amount is unrecorded in a gap');
  } finally { delete host.__tradeDrillPresentation; await vite.close(); }
});

test('an Other node and ribbon expose all 5,000 native contributors beyond the quick tooltip', () => {
  const data = decodeTrade(reply(5000));
  const diagram = buildDiagram(data.flows, 'current', 0, 5, 2);
  const selections = nodeSelections(data.flows, diagram.nodes);
  const source = diagram.nodes.find(node => node.side === 'source' && node.identity == null)!;
  const target = diagram.nodes.find(node => node.side === 'target')!;
  const selected = selections.get(source.id)!;
  const full = selectedContributors(data.flows, selected);
  assert.equal(full.length, 4998);
  assert.equal(new Set(full.map(flow => flow.key)).size, 4998);
  assert.ok(full.some(flow => flow.sellerIdentity === identity(4999)));
  assert.deepEqual(selectedContributors(data.flows, linkSelection(selected, selections.get(target.id)!, 7)), full);
  assert.deepEqual(selectedContributors(data.flows, readSelection(selectionToken(selected, CONTEXT), CONTEXT)!), full);
  assert.equal(contributionAmounts(full, 'current', 0, 60, data.reply.known).every(item => item.raw === 0n), true);
  assert.ok(parseStoredOptions({ tradeSelection: selectionToken(selected, CONTEXT) }));
  assert.equal(resolveOptions(OPTIONS, { tradeSelection: selectionToken(selected, CONTEXT) }).tradeSelection, selectionToken(selected, CONTEXT));
});

test('Other keeps negative, zero and inactive rows, with exact amounts and positive-only history', () => {
  const item = reply(6);
  item.flows[3].deliveredRaw = ['-9007199254740993'];
  item.flows[4].deliveredRaw = ['0'];
  item.flows[5].firstHours = [48]; item.flows[5].lastHours = [71];
  const data = decodeTrade(item);
  const diagram = buildDiagram(data.flows, 'current', 0, 5, 1);
  const other = diagram.nodes.find(node => node.side === 'source' && !node.identity)!;
  const full = selectedContributors(data.flows, nodeSelections(data.flows, diagram.nodes).get(other.id)!);
  assert.equal(full.length, 5);
  assert.deepEqual(contributionAmounts(full, 'current', 0, 5, item.known).map(item => item.raw), [5n, 4n, -9007199254740993n, 0n, 0n]);
  assert.equal(contributionAmounts(full, 'current', 0, 80, item.known).every(item => item.raw === null), true);
  assert.deepEqual(contributionHistory(data.flows[3], item.known), [{ firstHour: 0, lastHour: 71, value: 0 }]);
  const known = observedTradeSpans(item.known, [{ firstHour: 24, lastHour: 47 }]);
  assert.deepEqual(contributionHistory(data.flows[0], known), [{ firstHour: 0, lastHour: 23, value: 6 },
    { firstHour: 48, lastHour: 71, value: 0 }]);
  assert.equal(contributionAmounts(full, 'current', 0, 30, known).every(item => item.raw === null), true);
});

test('the full inventory remains selectable when every flow is nonpositive and no Other geometry exists', () => {
  const item = reply(2);
  item.flows[0].deliveredRaw = ['-10']; item.flows[1].deliveredRaw = ['0'];
  const data = decodeTrade(item);
  assert.equal(buildDiagram(data.flows, 'current', 0, 5, 12).nodes.length, 0);
  const selected = readSelection(selectionToken({ kind: 'all' }, CONTEXT), CONTEXT)!;
  assert.deepEqual(contributionAmounts(selectedContributors(data.flows, selected), 'current', 0, 5, item.known).map(item => item.raw), [-10n, 0n]);
  item.flows[0].deliveredRaw = ['10']; item.flows[1].deliveredRaw = ['-5'];
  const mixed = decodeTrade({ ...item });
  assert.equal(buildDiagram(mixed.flows, 'current', 0, 5, 12).nodes.some(node => !node.identity), false);
  assert.equal(selectedContributors(mixed.flows, selected).length, 2);
});

test('a saved native cohort refuses matching slot and resource IDs in a different campaign or branch', () => {
  const selection = { kind: 'link', sources: [identity(0)], targets: [identity(2)], resource: 7 } as const;
  const token = selectionToken(selection, CONTEXT);
  assert.deepEqual(readSelection(token, CONTEXT), selection);
  assert.equal(readSelection(token, { ...CONTEXT, campaignKey: 'another-campaign' }), null);
  assert.equal(readSelection(token, { ...CONTEXT, branchId: 'another-branch' }), null);
});

test('reused tags keep distinct overlapping native keys and supplying label dates', () => {
  const item = reply(2);
  item.countries = ['Other', 'Same tag'];
  item.countryIdentities = [{ tag: 'Other', runs: [{ firstHour: 0, lastHour: 71, slot: 2, countryId: 3 }] },
    { tag: 'Same tag', runs: [{ firstHour: 0, lastHour: 23, slot: 0, countryId: 1 }, { firstHour: 24, lastHour: 71, slot: 1, countryId: 2 }] }];
  item.flows[0].seller = 1; item.flows[0].buyer = 0; item.flows[0].sellerTags = [1]; item.flows[0].buyerTags = [0]; item.flows[0].lastHours = [23];
  item.flows[1].seller = 1; item.flows[1].buyer = 0; item.flows[1].sellerTags = [1]; item.flows[1].buyerTags = [0]; item.flows[1].firstHours = [24];
  const data = decodeTrade(item);
  const selection = { kind: 'node', side: 'source', holders: [identity(0), identity(1)] } as const;
  const full = selectedContributors(data.flows, selection);
  assert.equal(full.length, 2);
  assert.notEqual(full[0].key, full[1].key);
  assert.deepEqual(full.map(flow => flowCountryAt(flow, 'seller', 40)?.hour), [23, 40]);
  assert.deepEqual(selectedContributors(data.flows, { ...selection, holders: [identity(1)] }), [full[1]]);
});

test('Total and Average history track their selected basis, and picking a historical day gives the same exact measure', () => {
  const item = reply(1);
  item.flows[0] = { ...item.flows[0], firstHours: [0, 24], lastHours: [23, 47], deliveredRaw: ['10', '20'],
    sellerTags: [0, 0], buyerTags: [1, 1], factories: [1, 1], convoysAssigned: [0, 0], convoysRequired: [0, 0], deals: [1, 1] };
  const data = decodeTrade(item);
  for (const [measure, from, at, geometry, exact] of [['total', 0, 47, 30, 30], ['average', 0, 35, 40 / 3, 13],
    ['total', 24, 35, 10, 10], ['average', 24, 35, 20, 20]] as const) {
    const model = contributionMeasureModel(data.flows, measure, from, item.known);
    let end: number | null = null;
    forEachContributionMeasureSegment(model, 30, at, 1, piece => { if (piece.hours.at(-1) === at) end = piece.bands[0].high.at(-1)!; });
    assert.ok(Math.abs(end! - geometry) < 1e-9);
    assert.ok(contributionMeasureExtent(model, 30, at)![1] >= end!);
    const picked = reduceDrill({ breakdown: 'contributions', spec: false, follow: true, hour: null }, { type: 'pick', hour: at + 0.7 });
    const amount = contributionAmounts(data.flows, measure, from, picked.hour!, item.known)[0].raw;
    assert.equal(amount, BigInt(exact));
  }
  const known = observedTradeSpans(item.known, [{ firstHour: 24, lastHour: 47 }]);
  const model = contributionMeasureModel(data.flows, 'total', 0, known);
  const spans: [number, number][] = [];
  forEachContributionMeasureSegment(model, 20, 55, 5, piece => spans.push([piece.hours[0], piece.hours.at(-1)!]));
  assert.equal(spans.some(([first, last]) => first < 48 && last > 23), false);
});

test('5,000 cumulative contributors retain sparse source histories and stream only adjacent geometry', () => {
  const data = decodeTrade(reply(5000));
  const model = contributionMeasureModel(data.flows, 'average', 0, data.reply.known);
  assert.equal(model.flows, data.flows);
  assert.equal(model.hours.length, 5);
  let pieces = 0;
  forEachContributionMeasureSegment(model, 0, 47, 12, piece => {
    pieces++;
    assert.equal(piece.bands.length, 5000);
    assert.ok(piece.hours.length <= 2);
    assert.ok(piece.bands.every(band => band.high.length <= 2));
  });
  assert.ok(pieces > 0 && pieces < 10);
});

test('hourly changes across 50 days and 5,000 contributors draw pixel-budgeted geometry, not every native boundary', () => {
  const item = reply(1);
  item.known = [{ firstHour: 0, lastHour: 1199 }];
  const hours = Array.from({ length: 1200 }, (_, hour) => hour);
  item.flows[0] = { ...item.flows[0], firstHours: hours, lastHours: hours,
    deliveredRaw: hours.map(hour => String(10 + hour % 2)), sellerTags: hours.map(() => 0), buyerTags: hours.map(() => 1),
    factories: hours.map(() => 1), convoysAssigned: hours.map(() => 0), convoysRequired: hours.map(() => 0), deals: hours.map(() => 1) };
  const original = decodeTrade(item).flows[0];
  const flows = Array.from({ length: 5000 }, (_, index) => ({ ...original, key: `flow-${index}`, sellerIdentity: identity(index) }));
  const model = contributionMeasureModel(flows, 'total', 0, item.known);
  assert.equal(model.flows, flows);
  assert.equal(model.hours.length, 1201);
  assert.equal(model.guide.bands.length, 0);
  assert.equal(model.upper.length, model.hours.length);
  assert.equal(model.work.prefixWalks, flows.length);
  assert.ok(model.work.eventUpdates <= flows.length * (original.runs.length + 1));
  assert.equal(model.work.groupEvaluations, 0);
  let pieces = 0;
  let last = 0;
  forEachContributionMeasureSegment(model, 0, 1199, 24, piece => {
    pieces++;
    assert.equal(piece.bands.length, 5000);
    assert.equal(piece.hours.length, 2);
    last = piece.hours[1];
    const high = piece.bands.at(-1)!.high[1];
    assert.ok(contributionMeasureExtent(model, piece.hours[0], piece.hours[1])![1] >= high);
  });
  assert.equal(last, 1199);
  assert.ok(pieces > 0 && pieces < 100);
  assert.ok(contributionMeasureExtent(model, 1198.25, 1198.75)![1] > 0);
});

test('staggered 5,000 one-run native histories prepare scalar guides from source events rather than all-flow boundary walks', () => {
  const original = decodeTrade(reply(1)).flows[0];
  const flows = Array.from({ length: 5000 }, (_, index) => ({ ...original, key: `staggered-${index}`,
    sellerIdentity: identity(index), runs: [{ ...original.runs[0], firstHour: index, lastHour: index + 47 }] }));
  const known = [{ firstHour: 0, lastHour: 5047 }];
  for (const measure of ['total', 'average'] as const) {
    const model = contributionMeasureModel(flows, measure, 0, known);
    assert.ok(model.hours.length >= flows.length);
    assert.equal(model.work.prefixWalks, flows.length);
    assert.ok(model.work.eventUpdates <= flows.length * 3);
    assert.equal(model.work.groupEvaluations, 0);
    assert.equal(model.guide.bands.length, 0);
  }
});

test('event coefficients, positive crossings and unlike average denominators match native prefixes across held gaps and selected origins', () => {
  const item = reply(2);
  for (const [index, first, last, raw] of [[0, [0, 10, 20], [5, 15, 35], ['4', '8', '-2']],
    [1, [2, 11, 24], [6, 18, 45], ['-3', '7', '0']]] as const) {
    item.flows[index] = { ...item.flows[index], firstHours: [...first], lastHours: [...last], deliveredRaw: [...raw],
      sellerTags: first.map(() => index), buyerTags: first.map(() => 2), factories: first.map(() => 1),
      convoysAssigned: first.map(() => 0), convoysRequired: first.map(() => 0), deals: first.map(() => 1) };
  }
  const data = decodeTrade(item);
  const known = observedTradeSpans(item.known, [{ firstHour: 23, lastHour: 27 }]);
  for (const measure of ['total', 'average'] as const) for (const from of [0, 4, 12]) {
    const model = contributionMeasureModel(data.flows, measure, from, known);
    assert.equal(model.work.prefixWalks, data.flows.length);
    for (let index = 0; index < model.hours.length; index++) {
      const hour = model.hours[index];
      const bounds = model.guide.bounds[index];
      if (!bounds) continue;
      const expected = stackEdges(data.flows.map(flow => Math.max(0, flowValue(flow, measure, from, hour)))).at(-1)!.high;
      assert.ok(Math.abs(bounds[1] - expected) < 1e-9, `${measure} from ${from} at ${hour}: ${bounds[1]} versus ${expected}`);
      const next = model.hours[index + 1];
      if (next !== undefined && model.guide.bounds[index + 1]) {
        const middle = (hour + next) / 2;
        const high = stackEdges(data.flows.map(flow => Math.max(0, flowValue(flow, measure, from, middle)))).at(-1)!.high;
        assert.ok(contributionMeasureExtent(model, hour, middle)![1] + 1e-9 >= high);
      }
    }
    const selected = contributionAmounts(data.flows, measure, from, 30, known);
    assert.equal(selected.some(part => part.raw === null), false);
  }
  assert.deepEqual(contributionAmounts(data.flows, 'current', 0, 30, known).map(part => part.raw), [-2n, 0n]);
  assert.equal(contributionAmounts(data.flows, 'current', 0, 25, known).every(part => part.raw === null), true);
});

test('Average extent covers clipped fractional prefixes at the selected origin and keeps earlier zero history flat', () => {
  const data = decodeTrade(reply(2));
  const model = contributionMeasureModel(data.flows, 'average', 12, data.reply.known);
  const extent = contributionMeasureExtent(model, 11.25, 11.75)!;
  let pieces = 0;
  forEachContributionMeasureSegment(model, 11.25, 11.75, 1, piece => {
    pieces++;
    for (const high of piece.bands.at(-1)!.high) {
      assert.equal(high, 3);
      assert.ok(extent[1] >= high);
    }
  });
  assert.ok(pieces > 0);
  assert.deepEqual(extent, [0, 3]);
  assert.deepEqual(contributionMeasureExtent(model, 0, 10.75), [0, 0]);
  assert.deepEqual(contributionAmounts(data.flows, 'average', 12, 12, data.reply.known).map(part => part.raw), [2n, 1n]);
});


registerWidgets([definition]);
test('campaign drill Follow uses the actual aligned clock; day pick and Pin preserve branch-relative values and history', () => {
  const track = { id: 't', axisOffset: 1000, branch: { id: 'b', firstHour: 2000, lastHour: 9999 }, source: { metadata: { campaignKey: CONTEXT.campaignKey } } } as BoundTrack;
  const source: WidgetSpec = { id: 'w', kind: 'trade-flows', configVersion: 1, title: 'Trade flows', campaign: 'tracks', trackIds: ['t'],
    countries: { kind: 'all' }, time: { mode: 'range', firstOffset: 500, lastOffset: 6000 }, options: { measure: 'total', drill: 'popout' } };
  const selection = { kind: 'node', side: 'source', holders: [identity(7)] } as const;
  const opened = reduceDrill(CLOSED, { type: 'open', breakdown: 'contributions', hour: 9999, follow: true });
  const at = (state: typeof opened, clock: number) => regionHour({ widget: source, action: drillAction(state) }, track, [track], clock as AxisHour);
  assert.equal(at(opened, 3000.8), 4000);
  const picked = reduceDrill(opened, { type: 'pick', hour: 5500.8 });
  assert.equal(at(picked, 3500), 5500);
  assert.equal(at(reduceDrill(picked, { type: 'follow' }), 3600), 4600);
  const pinned = pinContributions(source, track, selection, gatedTime({ mode: 'pin', windows: { t: { firstOffset: 3500, lastOffset: 3500 } } }), [2500, 8000]);
  assert.deepEqual(pinned.time.value, { mode: 'pin', windows: { t: { firstOffset: 3500, lastOffset: 3500 } } });
  assert.deepEqual(readSelection(pinned.options?.tradeSelection, tradeContext(track)), selection);
  assert.deepEqual(contributionExtent(pinned.options?.tradeExtent, { firstHour: 20_000 }), [20_500, 26_000]);
  assert.equal(pinned.options?.measure, 'total');
  assert.deepEqual(pinContributions(source, track, selection, gatedTime({ mode: 'follow' }), [2500, 8000]).time.value, { mode: 'follow' });
});

test('a country view saves one native holder, selects both its sides and pins for that country', () => {
  const track = { id: 't', axisOffset: 0, branch: { id: 'b', firstHour: 2000, lastHour: 9999 }, source: { metadata: { campaignKey: CONTEXT.campaignKey } } } as BoundTrack;
  const source: WidgetSpec = { id: 'w', kind: 'trade-flows', configVersion: 1, title: 'Trade flows', campaign: 'tracks', trackIds: ['t'],
    countries: { kind: 'all' }, time: { mode: 'campaign' }, options: { measure: 'current' } };
  const selection = { kind: 'country', holder: identity(1) } as const;
  assert.deepEqual(readSelection(selectionToken(selection, CONTEXT), CONTEXT), selection);
  assert.equal(readSelection(selectionToken({ kind: 'country', holder: '' } as never, CONTEXT), CONTEXT), null);
  const data = decodeTrade(reply(3));
  assert.deepEqual(selectedContributors(data.flows, selection).map(flow => flow.key), [data.flows[1].key]);
  assert.equal(selectedContributors(data.flows, { kind: 'country', holder: identity(3) }).length, 3);
  const pinned = pinContributions(source, track, selection, gatedTime({ mode: 'pin', windows: { t: { firstOffset: 3000, lastOffset: 3000 } } }), [2000, 9999], 'Country 00001');
  assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['Country 00001'] }, 'the pin reads and names that country');
  assert.equal(pinned.title, 'Trade flows: Country trade');
  assert.equal(pinned.options?.show, 'contributions');
  assert.deepEqual(readSelection(pinned.options?.tradeSelection, tradeContext(track)), selection);
  const index = countryIdentityIndex([{ tag: 'X', identities: [{ firstHour: 0, lastHour: 50, slot: 1, countryId: 2 }, { firstHour: 51, lastHour: 99, slot: 5, countryId: 6 }] }]);
  assert.equal(contributionCountryTag(index, 'X', { kind: 'country', holder: identity(1) }, 20), 'X');
  assert.equal(contributionCountryTag(index, 'X', { kind: 'country', holder: identity(1) }, 70), null, 'a later holder of the tag is not this country');
});


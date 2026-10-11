import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveCountries, worldFilter } from '../../dashboard/groups.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { definition, OPTIONS } from './definition.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { ResourceInfo, TradeFlow, TradeFlowsReply } from '../../generated/dashboard';
import { buildDiagram, campaignScale, coverageNote, decodeTrade, emptyMessage, flowValue, knownSpanAt, leftOutAt, measureAmount, measureRows, OTHER, resourceColours, selectFlows, selectionMessage,
  sideRanks, tradeScope, flowCountryAt, identityCountryAt, unknownNote, unrecordedMessage, validateTradeFlowsReply, resourceName, measureNote } from './tradeData.ts';

const resource = (id: number, token: string): ResourceInfo => ({ id, token, ordinal: 1, name: null, icon: null });
/** A flow from runs of [first, last, delivered raw, factories, convoys assigned, required]. */
const flow = (seller: number, buyer: number, id: number, runs: [number, number, string, number?, number?, number?][]): TradeFlow => ({
  seller, buyer, resource: id, sellerIdentity: JSON.stringify([seller, seller + 1]), buyerIdentity: JSON.stringify([buyer, buyer + 1]),
  sellerTags: runs.map(() => seller), buyerTags: runs.map(() => buyer),
  firstHours: runs.map(run => run[0]), lastHours: runs.map(run => run[1]), deliveredRaw: runs.map(run => run[2]),
  factories: runs.map(run => run[3] ?? 1), convoysAssigned: runs.map(run => run[4] ?? 0), convoysRequired: runs.map(run => run[5] ?? 0), deals: runs.map(() => 1),
});

/** GER sells ITA steel at 10/day for hours 0–47, then 20/day for 48–71; ITA sells
 * SPR oil at 5/day for 24–47; hours 72–95 are not recorded. */
function reply(countries = ['GER', 'ITA', 'SPR']): TradeFlowsReply {
  return {
    branchId: '1', decimals: 5, resources: [resource(1, 'steel'), resource(2, 'oil')], countries,
    countryIdentities: countries.map((tag, slot) => ({ tag, runs: [{ firstHour: 0, lastHour: 575, slot, countryId: slot + 1 }] }))
      .sort((a, b) => a.tag < b.tag ? -1 : a.tag > b.tag ? 1 : 0),
    reads: [0, 24, 48], known: [{ firstHour: 0, lastHour: 71 }, { firstHour: 96, lastHour: 119 }], partial: [{ firstHour: 24, lastHour: 47, deals: 2 }],
    flows: [flow(0, 1, 1, [[0, 47, '1000000', 2, 3, 4], [48, 71, '2000000', 2, 3, 4]]), flow(1, 2, 2, [[24, 47, '500000', 1]])],
    warnings: [],
  };
}

test('trade selection accepts every recorded country', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD${index}`);
  const resolved = resolveCountries({ kind: 'tags', tags }, [], undefined, 119, definition.countries.max);
  assert.deepEqual([resolved.tags, resolved.dropped], [tags, 0]);
});

test('trade labels keep content names and ordinals without exposing unresolved tokens', () => {
  assert.equal(resourceName(resource(1, 'private_mod_resource')), 'Resource name unavailable');
  assert.equal(resourceName({ ...resource(1, 'private_mod_resource'), ordinal: 2 }), 'Resource name unavailable (2)');
  assert.equal(resourceName({ ...resource(1, 'private_mod_resource'), name: 'Modded resource' }), 'Modded resource');
  assert.equal(measureNote('current'), 'Per day');
  assert.equal(measureNote('total'), 'Total delivered');
  assert.equal(measureNote('average'), 'Average per trading day');
});

test('countries per side accepts counts above forty and one, keeping the default twelve', () => {
  assert.equal(resolveOptions(OPTIONS, {}).perSide, 12);
  for (const perSide of [1, 80]) assert.equal(resolveOptions(OPTIONS, { perSide }).perSide, perSide);
});

test('a reply is checked for parallel runs in order with exact amounts', () => {
  assert.doesNotThrow(() => validateTradeFlowsReply(reply()));
  const broken = [
    (item: TradeFlowsReply) => { item.flows[0].lastHours.pop(); },
    (item: TradeFlowsReply) => { item.flows[0].firstHours[1] = 40; },
    (item: TradeFlowsReply) => { item.flows[0].deliveredRaw[0] = '1.5'; },
    (item: TradeFlowsReply) => { item.flows[0].resource = 9; },
    (item: TradeFlowsReply) => { item.flows[0].buyer = 3; },
    (item: TradeFlowsReply) => { item.known.reverse(); },
    (item: TradeFlowsReply) => { item.flows[0].sellerTags.pop(); },
    (item: TradeFlowsReply) => { item.flows[0].sellerIdentity = 'tag:GER'; },
    (item: TradeFlowsReply) => { item.countryIdentities[0].runs[0].countryId = 77; },
    (item: TradeFlowsReply) => { item.flows[0].deliveredRaw[0] = '9223372036854775808'; },
  ];
  for (const breakIt of broken) {
    const item = reply();
    breakIt(item);
    assert.throws(() => validateTradeFlowsReply(item));
  }
});

test('the measures: the amount at an hour, the total so far and the average per day while trading', () => {
  const [steel, oil] = decodeTrade(reply()).flows;
  assert.equal(flowValue(steel, 'current', 0, 50), 20);
  assert.equal(flowValue(oil, 'current', 0, 50), 0);
  // 48 hours at 10 and 12 at 20: 20 + 10 delivered.
  assert.equal(flowValue(steel, 'total', 0, 59), 30);
  assert.equal(flowValue(steel, 'total', 24, 59), 20);
  // Average over the 60 hours traded: 30 over 2.5 days.
  assert.equal(flowValue(steel, 'average', 0, 59), 12);
  assert.equal(flowValue(oil, 'average', 0, 200), 5);
  assert.equal(flowValue(oil, 'total', 0, 10), 0);
});

test('exact amounts and terms for tooltips', () => {
  const data = decodeTrade(reply());
  const [steel, oil] = data.flows;
  assert.equal(measureAmount([steel], 'total', 0, 59), 3_000_000n);
  assert.equal(measureAmount([steel, oil], 'current', 0, 30), 1_500_000n);
  // Each flow's own average, summed: steel 12 over hours 0–59 and oil 5.
  assert.equal(measureAmount([steel, oil], 'average', 0, 59), 1_700_000n);
  const current = measureRows([steel], 'current', 0, 50, 5);
  assert.deepEqual(current.map(row => [row.label, row.value]), [['Per day', '20'], ['Civilian factories paid', '2'], ['Convoys used', '3 of 4']]);
  assert.equal(measureRows([oil], 'current', 0, 30, 5)[2].value, 'None needed');
  const total = measureRows([steel], 'total', 0, 59, 5);
  assert.deepEqual(total.map(row => [row.label, row.value]),
    [['Traded', '30'], ['Days traded', '2.5 days'], ['Civilian factories paid, on average', '2'], ['Convoys used, on average', '3 of 4']]);
  assert.equal(measureRows([steel, oil], 'average', 0, 59, 5)[1].value, '2');
});

test('chosen countries show trade touching them or only between them, cut to membership', () => {
  const data = decodeTrade(reply());
  const keys = (flows: { seller: string; buyer: string }[]) => flows.map(item => `${item.seller}>${item.buyer}`);
  assert.deepEqual(keys(selectFlows(data, { kind: 'all' }, null)), ['GER>ITA', 'ITA>SPR']);
  assert.deepEqual(keys(selectFlows(data, { kind: 'all' }, new Set([2]))), ['ITA>SPR']);
  assert.deepEqual(keys(selectFlows(data, { kind: 'countries', tags: ['SPR'], membership: null, between: false }, null)), ['ITA>SPR']);
  assert.deepEqual(keys(selectFlows(data, { kind: 'countries', tags: ['ITA', 'SPR'], membership: null, between: true }, null)), ['ITA>SPR']);
  // GER counts only for hours 0–29: its sales end there; ITA never counts.
  const membership = new Map([['GER', [{ firstHour: 0, lastHour: 29 }]]]);
  const clipped = selectFlows(data, { kind: 'countries', tags: ['GER', 'ITA'], membership, between: false }, null);
  assert.deepEqual(clipped[0].runs.map(run => [run.firstHour, run.lastHour]), [[0, 29]]);
  assert.equal(flowValue(clipped[0], 'total', 0, 100), 12.5);
  assert.equal(clipped.length, 1);
  // Hours the group's membership was not recorded count nothing: A is human over 0–47
  // selling 24 a day, with 24–47 unobserved, so 24 is counted, not 48.
  const item: TradeFlowsReply = { ...reply(['A', 'B']), flows: [flow(0, 1, 1, [[0, 47, '2400000']])] };
  const unobserved = [{ firstHour: 24, lastHour: 47 }];
  const counted = selectFlows(decodeTrade(item), { kind: 'countries', tags: ['A'], membership: new Map([['A', [{ firstHour: 0, lastHour: 47 }]]]), unobserved, between: false }, null);
  assert.equal(flowValue(counted[0], 'total', 0, 47), 24);
  // Between: both must count at once.
  const both = new Map([['ITA', [{ firstHour: 30, lastHour: 40 }]], ['SPR', [{ firstHour: 35, lastHour: 60 }]]]);
  const between = selectFlows(data, { kind: 'countries', tags: ['ITA', 'SPR'], membership: both, between: true }, null);
  assert.deepEqual(between[0].runs.map(run => [run.firstHour, run.lastHour]), [[35, 40]]);
});

test('past the limit a side sums the rest into one node, leaving nothing out', () => {
  const countries = ['A', 'B', 'C', 'D', 'X'];
  const item: TradeFlowsReply = { ...reply(countries), flows: [
    flow(0, 4, 1, [[0, 10, '400000']]), flow(1, 4, 1, [[0, 10, '300000']]), flow(2, 4, 1, [[0, 10, '200000']]), flow(3, 4, 1, [[0, 10, '100000']]),
  ] };
  const flows = decodeTrade(item).flows;
  const diagram = buildDiagram(flows, 'current', 0, 5, 2);
  const sources = diagram.nodes.filter(node => node.side === 'source');
  assert.deepEqual(sources.map(node => node.tag ?? OTHER).sort(), ['A', 'B', OTHER].sort());
  const other = sources.find(node => !node.tag)!;
  assert.deepEqual(other.members, ['[2,3]', '[3,4]']);
  assert.ok(Math.abs(other.value - 3) < 1e-9);
  assert.ok(Math.abs(diagram.total - 10) < 1e-9);
  // One country past the limit is shown itself rather than as "1 other".
  assert.equal(buildDiagram(flows, 'current', 0, 5, 3).nodes.filter(node => node.side === 'source' && !node.tag).length, 0);
  // Nothing trades at hour 20.
  assert.equal(buildDiagram(flows, 'current', 0, 20, 2).links.length, 0);
});

test('tags are opaque: separators, spaces and dynamic tags never blur two ribbons', () => {
  // Joined naively, "A>t:B" selling to "C" and "A" selling to "B>t:C" could share an id.
  const item: TradeFlowsReply = { ...reply(['A', 'A>t:B', 'B>t:C', 'C', 'D01', 'It aly']), flows: [
    flow(1, 3, 1, [[0, 10, '100000']]), flow(0, 2, 1, [[0, 10, '200000']]), flow(4, 5, 1, [[0, 10, '300000']]),
  ] };
  const diagram = buildDiagram(decodeTrade(validateTradeFlowsReply(item)).flows, 'current', 0, 5, 12);
  assert.equal(new Set(diagram.links.map(link => link.id)).size, 3);
  assert.deepEqual(diagram.nodes.filter(node => node.side === 'target').map(node => node.tag).sort(), ['B>t:C', 'C', 'It aly']);
  const scoped = selectFlows(decodeTrade(item), { kind: 'countries', tags: ['It aly'], membership: null, between: false }, null);
  assert.deepEqual(scoped.map(each => each.seller), ['D01']);
});

test('each side keeps one order for the whole time shown, largest first', () => {
  const flows = decodeTrade(reply()).flows;
  // Over everything, GER sold 40 and ITA 5; at the end ITA sells nothing, yet the order holds.
  const ranks = sideRanks(flows, 0, 119);
  assert.deepEqual([...ranks.source], [['[0,1]', 0], ['[1,2]', 1]]);
  assert.deepEqual([...ranks.target], [['[1,2]', 0], ['[2,3]', 1]]);
});

test('one scale all campaign: the most traded at once, the final total, the largest average', () => {
  const flows = decodeTrade(reply()).flows;
  assert.equal(campaignScale(flows, 'current', 0, 119), 20);
  assert.equal(campaignScale(flows, 'current', 0, 47), 15);
  assert.equal(campaignScale(flows, 'total', 0, 119), 40 + 5);
  assert.ok(campaignScale(flows, 'average', 0, 119) >= flowValue(flows[0], 'average', 0, 119) + 5 - 1e-9);
  // 1, 100 then 1 a day: the running average peaks between reads, at 50.5 at hour 47.
  const peaked = decodeTrade({ ...reply(), flows: [flow(0, 1, 1, [[0, 23, '100000'], [24, 47, '10000000'], [48, 71, '100000']])] }).flows;
  assert.equal(flowValue(peaked[0], 'average', 0, 47), 50.5);
  assert.equal(campaignScale(peaked, 'average', 0, 71), 50.5);
});

test('unknown hours are found and said, never counted as no trade', () => {
  const known = reply().known;
  assert.equal(knownSpanAt(known, 71), 0);
  assert.equal(knownSpanAt(known, 80), -1);
  assert.equal(knownSpanAt(known, 100), 1);
  assert.equal(unknownNote(known, 0, 71), undefined);
  assert.equal(unknownNote(known, 0, 119), 'Trade data is missing for 1 day.');
  // Every measure says a time with nothing recorded is not recorded, never no trade.
  const gappy = [{ firstHour: 0, lastHour: 23 }, { firstHour: 48, lastHour: 71 }];
  for (const measure of ['total', 'average'] as const) assert.equal(unrecordedMessage(gappy, measure, 24, 47), 'Trade deals were not recorded in this time.');
  assert.equal(unrecordedMessage(gappy, 'total', 0, 47), null);
  assert.equal(unrecordedMessage(gappy, 'current', 0, 30), 'Trade deals were not recorded at this time.');
  assert.equal(unrecordedMessage(gappy, 'current', 0, 50), null);
});

test('an empty diagram over an incompletely recorded time never says there was no trade', () => {
  const item = reply();
  // Hours 24–47 had deals left out: no attributed trade, for every measure.
  assert.equal(emptyMessage(item, 'current', 0, 30), 'No trade the recording could attribute at this moment.');
  for (const measure of ['total', 'average'] as const) {
    assert.equal(emptyMessage(item, measure, 24, 47), 'No trade the recording could attribute in this time.');
    // Hours 72–95 were not recorded.
    assert.equal(emptyMessage(item, measure, 60, 100), 'No trade in the recorded part of this time.');
    assert.equal(emptyMessage(item, measure, 96, 119), 'No trade in this time.');
    // A group counted at each hour whose membership was not recorded at all, or in part.
    assert.equal(emptyMessage(item, measure, 96, 119, [{ firstHour: 90, lastHour: 130 }]), 'This group’s members were not recorded in this time.');
    assert.equal(emptyMessage(item, measure, 96, 119, [{ firstHour: 100, lastHour: 101 }]), 'No trade in the recorded part of this time.');
  }
  assert.equal(emptyMessage(item, 'current', 0, 50), 'No trade at this moment.');
});

test('convoys needed now and then are said, however small their average', () => {
  // One convoy needed and used on the first day of 24, none after.
  const item: TradeFlowsReply = { ...reply(), flows: [flow(0, 1, 1, [[0, 23, '100000', 1, 1, 1], [24, 575, '100000', 0, 0, 0]])] };
  const rows = measureRows(decodeTrade(item).flows, 'average', 0, 575, 5);
  assert.deepEqual(rows.slice(2).map(row => row.value), ['<0.1', '<0.1 of <0.1']);
  const none: TradeFlowsReply = { ...reply(), flows: [flow(0, 1, 1, [[0, 575, '100000', 0, 0, 0]])] };
  assert.deepEqual(measureRows(decodeTrade(none).flows, 'total', 0, 575, 5).slice(2).map(row => row.value), ['0', 'None needed']);
});

test('an unrecorded human roster is unknown before an empty selection stops the request', () => {
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [], unobserved: { humans: [{ firstHour: 0, lastHour: 575 }], factions: [], wars: [] }, warnings: [] };
  const resolved = resolveCountries(selection, [], groups, 575, 256);
  assert.deepEqual(resolved.tags, []);
  const atMoment = worldFilter(selection, [], groups, 300, 575);
  assert.equal(selectionMessage(resolved, atMoment, 'current', 0, 300), 'Group membership is unavailable at this time.');
  const ever: CountrySelection = { ...selection, membership: 'ever' };
  const everReason = 'Group membership is incomplete for this campaign.';
  const everResolved = resolveCountries(ever, [], groups, 575, 256);
  const everMoment = worldFilter(ever, [], groups, 300, 575);
  assert.equal(everResolved.reason, everReason);
  assert.equal(everMoment.reason, everReason);
  assert.equal(selectionMessage(everResolved, everMoment, 'current', 0, 300), everReason);
  const recorded = { ...groups, unobserved: { ...groups.unobserved, humans: [] } };
  assert.equal(selectionMessage(resolveCountries(selection, [], recorded, 575, 256), worldFilter(selection, [], recorded, 300, 575), 'current', 0, 300),
    'This group has no members at this moment.');
});

test('a ribbon sums 24 averages of one factory and convoy on the first of 24 days before rounding', () => {
  const countries = [...Array.from({ length: 26 }, (_, index) => `Seller ${index}`), 'Buyer'];
  const item: TradeFlowsReply = { ...reply(countries), flows: countries.slice(0, -1).map((_, index) => flow(index, 26, 1,
    [[0, 23, index < 2 ? '10000000' : '100000', 1, 1, 1], [24, 575, index < 2 ? '10000000' : '100000', 0, 0, 0]])) };
  const diagram = buildDiagram(decodeTrade(item).flows, 'current', 0, 575, 2);
  const ribbon = diagram.links.find(link => link.source === `s:${OTHER}`)!;
  assert.equal(ribbon.flows.length, 24);
  for (const measure of ['total', 'average'] as const) {
    assert.deepEqual(measureRows(ribbon.flows, measure, 0, 575, 5).slice(2).map(row => row.value), ['1', '1 of 1']);
  }
});

test('an empty each-hour group says how much membership was unrecorded in totals and averages', () => {
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [], unobserved: { humans: [{ firstHour: 0, lastHour: 299 }], factions: [], wars: [] }, warnings: [] };
  const resolved = resolveCountries(selection, [], groups, 575, 256);
  const atMoment = worldFilter(selection, [], groups, 575, 575);
  for (const measure of ['total', 'average'] as const) {
    assert.equal(selectionMessage(resolved, atMoment, measure, 0, 575, resolved.unobserved),
      'Group membership is missing for 12.5 days.');
    assert.equal(selectionMessage(resolved, atMoment, measure, 300, 575, resolved.unobserved), 'This group has no members at this moment.');
    assert.equal(selectionMessage(resolved, atMoment, measure, 250, 575, resolved.unobserved),
      'Group membership is missing for 2.1 days.');
    // A roster counted at the end does not depend on earlier missing hours.
    assert.equal(selectionMessage(resolved, atMoment, measure, 0, 575), 'This group has no members at this moment.');
  }
  assert.equal(selectionMessage(resolved, atMoment, 'current', 0, 575, resolved.unobserved), 'This group has no members at this moment.');
});

test('a ribbon rounds an exact half-tenth sum of averages once, independent of order', () => {
  // Eighteen averages of 1/24 sum to 0.75, whose floating sum lands below the tie.
  const item: TradeFlowsReply = { ...reply(), flows: Array.from({ length: 18 }, () => flow(0, 1, 1,
    [[0, 0, '100000', 1, 1, 1], [1, 23, '100000', 0, 0, 0]])) };
  const same = decodeTrade(item).flows;
  // Different trade durations must also sum before rounding: 1/3 + 1/15 + 1/20 = 0.45.
  const varied = decodeTrade({ ...reply(), flows: [3, 15, 20].map(hours => flow(0, 1, 1,
    [[0, 0, '100000', 1, 1, 1], [1, hours - 1, '100000', 0, 0, 0]])) }).flows;
  for (const [flows, expected] of [[same, '0.8'], [varied, '0.5'], [[...varied].reverse(), '0.5']] as const) {
    for (const measure of ['total', 'average'] as const) {
      assert.deepEqual(measureRows(flows, measure, 0, 23, 5).slice(2).map(row => row.value), [expected, `${expected} of ${expected}`]);
    }
  }
});

test('hours whose reads left deals out are said, never shown as the whole trade', () => {
  const item = reply();
  assert.equal(leftOutAt(item.partial, 30), 2);
  assert.equal(leftOutAt(item.partial, 50), 0);
  assert.equal(coverageNote(item, 'current', 0, 30),
    '2 trades could not be identified.');
  assert.equal(coverageNote(item, 'current', 0, 50), undefined);
  assert.equal(coverageNote(item, 'total', 0, 71), 'Some trades are missing for 1 day.');
  assert.equal(coverageNote(item, 'average', 0, 119),
    'Trade data is missing for 1 day. Some trades are missing for 1 day.');
  assert.throws(() => validateTradeFlowsReply({ ...reply(), partial: [{ firstHour: 5, lastHour: 4, deals: 1 }] }));
});

test('resource colours follow the game order, never the filter, and a ninth folds into other', () => {
  const resources = Array.from({ length: 10 }, (_, index) => resource(index + 1, `r${index}`));
  const order = new Map(resources.map((item, index) => [item.id, index]));
  const palette = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'];
  const all = resourceColours(resources, order, palette, 'other');
  assert.deepEqual([...all.values()], [...palette, 'other', 'other']);
  // Filtered to two, each keeps its own colour.
  const two = resourceColours([resources[5], resources[2]], order, palette, 'other');
  assert.deepEqual([two.get(3), two.get(6)], ['c', 'f']);
  // The ninth shown alone is still "Other", never the palette's first colour.
  assert.deepEqual([...resourceColours([resources[8]], order, palette, 'other').values()], ['other']);
});

/** Holder [7,11] changes X to Y; unrelated [9,31] takes X. Numeric histories remain
 * independent, with census names extending after their last trade. */
function renamedReply(): TradeFlowsReply {
  const old = { ...flow(1, 0, 1, [[0, 23, '1000000'], [24, 47, '2000000']]), sellerIdentity: '[7,11]', buyerIdentity: '[8,21]', sellerTags: [1, 2] };
  const replacement = { ...flow(1, 0, 1, [[24, 47, '3000000']]), sellerIdentity: '[9,31]', buyerIdentity: '[8,21]' };
  return { ...reply(['Buyer', 'X', 'Y']), flows: [old, replacement], countryIdentities: [
    { tag: 'Buyer', runs: [{ firstHour: 0, lastHour: 71, slot: 8, countryId: 21 }] },
    { tag: 'X', runs: [{ firstHour: 0, lastHour: 23, slot: 7, countryId: 11 }, { firstHour: 24, lastHour: 71, slot: 9, countryId: 31 }] },
    { tag: 'Y', runs: [{ firstHour: 24, lastHour: 71, slot: 7, countryId: 11 }] },
  ] };
}

test('renames keep weighted history, reused tags keep separate native holders and dated labels', () => {
  const data = decodeTrade(validateTradeFlowsReply(renamedReply()));
  assert.notEqual(data.flows[0].key, data.flows[1].key);
  assert.equal(measureAmount([data.flows[0]], 'average', 0, 47), 1_500_000n);
  assert.equal(measureAmount(data.flows, 'total', 0, 47), 6_000_000n);
  assert.equal(flowCountryAt(data.flows[0], 'seller', 23)?.tag, 'X');
  assert.equal(flowCountryAt(data.flows[0], 'seller', 29)?.tag, 'Y');
  assert.equal(identityCountryAt(data, '[9,31]', 23), null);
  assert.equal(identityCountryAt(data, '[7,11]', -1), null);
  assert.deepEqual(identityCountryAt(data, '[7,11]', 100), { identity: '[7,11]', tag: 'Y', hour: 71, firstHour: 24, lastHour: 71 });
  const nodes = buildDiagram(data.flows, 'total', 0, 47, 12).nodes.filter(node => node.side === 'source');
  assert.deepEqual(nodes.map(node => [node.identity, node.tag, node.factHour]), [['[7,11]', 'Y', 47], ['[9,31]', 'X', 47]]);
  const chosen = selectFlows(data, { kind: 'countries', tags: ['Y'], membership: null, between: false }, null);
  assert.deepEqual(chosen.map(item => item.key), [data.flows[0].key]);
  assert.equal(measureAmount(chosen, 'total', 0, 47), 3_000_000n);
});

test('end and ever groups select dated holders before including their old aliases; hourly groups still clip', () => {
  const data = decodeTrade(renamedReply());
  const groups: CountryGroupReply = { branchId: '1', humans: [{ tag: 'Y', firstHour: 24, lastHour: 71 }], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: data.reply.countryIdentities.map(({ tag, runs }) => ({ tag, identities: runs })),
    unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'end', aggregate: false };
  const choose = (picked: CountrySelection, roster = groups) => selectFlows(data, tradeScope(data, picked, resolveCountries(picked, [], roster, 71, Infinity), roster, 71, false), null);
  const end = choose(selection);
  assert.deepEqual(end.map(item => item.sellerIdentity), ['[7,11]']);
  assert.equal(measureAmount(end, 'total', 0, 47), 3_000_000n); // Includes X before the end member was called Y.
  const oldMembers = { ...groups, humans: [{ tag: 'X', firstHour: 0, lastHour: 23 }] };
  const ever = choose({ ...selection, membership: 'ever' }, oldMembers);
  assert.deepEqual(ever.map(item => item.sellerIdentity), ['[7,11]']); // X's later replacement was never a member.
  assert.equal(measureAmount(ever, 'average', 0, 47), 1_500_000n);
  const hourly = choose({ ...selection, membership: 'hour' });
  assert.deepEqual(hourly[0].runs.map(run => [run.firstHour, run.lastHour]), [[24, 47]]);
  assert.equal(measureAmount(hourly, 'total', 0, 47), 2_000_000n);
});

test('a missing end-group holder is unknown rather than selecting a previous or future holder', () => {
  const item = renamedReply();
  item.countryIdentities[2].runs[0].lastHour = 47;
  const data = decodeTrade(item);
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'end', aggregate: false };
  const groups: CountryGroupReply = { branchId: '1', humans: [{ tag: 'Y', firstHour: 24, lastHour: 71 }], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: item.countryIdentities.map(({ tag, runs }) => ({ tag, identities: runs })),
    unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  const scope = tradeScope(data, selection, resolveCountries(selection, [], groups, 71, Infinity), groups, 71, false);
  assert.ok(scope.kind === 'countries');
  assert.match(scope.identityNote ?? '', /Some trade could not be identified/);
  assert.equal(scope.countryIds?.size, 0);
  assert.equal(selectFlows(data, scope, null).length, 0);
});

test('identity lookup does not fill an unrecorded interval from a future rename', () => {
  const item = renamedReply();
  item.countryIdentities[1].runs[0].lastHour = 20;
  const data = decodeTrade(item);
  assert.deepEqual(identityCountryAt(data, '[7,11]', 22), { identity: '[7,11]', tag: 'X', hour: 20, firstHour: 0, lastHour: 20 });
  assert.equal(identityCountryAt(data, '[7,11]', 24)?.tag, 'Y');
  assert.throws(() => validateTradeFlowsReply(item)); // Numeric run cannot span that missing identity evidence.
});

test('five thousand native holders retain distinct histories and selected end-group identity', () => {
  const countries = Array.from({ length: 5000 }, (_, index) => `Mod${String(index).padStart(4, '0')}`);
  const item = { ...reply(countries), flows: countries.map((_, index) => flow(index, (index + 1) % countries.length, 1, [[0, 23, '100000']])) };
  const data = decodeTrade(validateTradeFlowsReply(item));
  assert.equal(new Set(data.flows.map(each => each.key)).size, 5000);
  const chosen = selectFlows(data, { kind: 'countries', tags: [countries[4999]], membership: null, countryIds: new Set(['[4999,5000]']), between: false }, null);
  assert.deepEqual(chosen.map(each => [each.sellerIdentity, each.buyerIdentity]), [['[4998,4999]', '[4999,5000]'], ['[4999,5000]', '[0,1]']]);
  assert.equal(buildDiagram(data.flows, 'total', 0, 23, 12).total, 5000);
});

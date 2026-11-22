import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeTrade } from './tradeData.ts';
import { bandGroups, countryParts, countryTotal, countryTrade, countryTradeTags, divergingExtent, mergedBands, mirrored, partnerFact } from './tradeCountry.ts';
import type { TradeFlowsReply } from '../../generated/dashboard';
import { assertLinearWork, measureWork } from '../../test-support/work.ts';

const A = '[1,1]', B = '[2,2]', C = '[3,3]';
const reply: TradeFlowsReply = { branchId: 'b', decimals: 0,
  countries: ['AAA', 'AAZ', 'BBB', 'CCC'],
  countryIdentities: [
    { tag: 'AAA', runs: [{ firstHour: 0, lastHour: 47, slot: 1, countryId: 1 }] },
    { tag: 'AAZ', runs: [{ firstHour: 48, lastHour: 95, slot: 1, countryId: 1 }] },
    { tag: 'BBB', runs: [{ firstHour: 0, lastHour: 95, slot: 2, countryId: 2 }] },
    { tag: 'CCC', runs: [{ firstHour: 0, lastHour: 95, slot: 3, countryId: 3 }] },
  ],
  resources: [{ id: 2, token: 'mod_oil', ordinal: 1, name: 'Oil', icon: null }, { id: 9, token: 'mod_steel', ordinal: 1, name: 'Steel', icon: null }],
  firstHour: 0, lastHour: 2000, known: [{ firstHour: 0, lastHour: 95 }], partial: [], warnings: [],
  flows: [
    { seller: 0, buyer: 2, sellerIdentity: A, buyerIdentity: B, resource: 9, firstHours: [0, 48], lastHours: [47, 71], deliveredRaw: ['10', '10'],
      sellerTags: [0, 1], buyerTags: [2, 2], factories: [1, 1], convoysAssigned: [0, 0], convoysRequired: [0, 0], deals: [1, 1] },
    { seller: 2, buyer: 0, sellerIdentity: B, buyerIdentity: A, resource: 2, firstHours: [24, 48], lastHours: [47, 71], deliveredRaw: ['5', '5'],
      sellerTags: [2, 2], buyerTags: [0, 1], factories: [1, 1], convoysAssigned: [0, 0], convoysRequired: [0, 0], deals: [1, 1] },
    { seller: 3, buyer: 0, sellerIdentity: C, buyerIdentity: A, resource: 9, firstHours: [0], lastHours: [23], deliveredRaw: ['0'],
      sellerTags: [3], buyerTags: [0], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] },
    { seller: 3, buyer: 2, sellerIdentity: C, buyerIdentity: B, resource: 2, firstHours: [0], lastHours: [95], deliveredRaw: ['7'],
      sellerTags: [3], buyerTags: [2], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] },
  ] };
const trade = decodeTrade(reply);

test('a country outside the chosen tags asks for its own aliases; a covered one reads the shown reply', () => {
  assert.equal(countryTradeTags(trade, null, A), null, 'every country’s trade is already read');
  assert.equal(countryTradeTags(trade, ['AAZ'], A), null, 'any alias asked for brings the native holder’s whole trade');
  assert.deepEqual(countryTradeTags(trade, ['BBB'], A), ['AAA', 'AAZ']);
  assert.deepEqual(countryTradeTags(trade, [], C), ['CCC']);
});

test('a country’s trade is what it bought and sold, in content resource order, for the chosen resources', () => {
  const all = countryTrade(trade, A, null);
  assert.deepEqual(all.exports.map(flow => flow.key), [JSON.stringify([A, B, 9])]);
  assert.deepEqual(all.imports.map(flow => [flow.sellerIdentity, flow.resource.id]), [[B, 2], [C, 9]]);
  assert.deepEqual(countryTrade(trade, A, new Set([9])).imports.map(flow => flow.sellerIdentity), [C]);
  assert.deepEqual(countryTrade(trade, B, null).imports.map(flow => flow.sellerIdentity), [C, A], 'oil before steel');
});

test('partners and resources list the deals running at the day or in the time, with exact amounts', () => {
  const { imports, exports } = countryTrade(trade, A, null);
  assert.deepEqual(countryParts(imports, 'imports', 'partner', 'current', 0, 10, reply.known).map(part => [part.id, part.raw]), [[C, 0n]]);
  assert.equal(countryTotal(imports, 'current', 0, 10, reply.known), 0n);
  assert.deepEqual(countryParts(imports, 'imports', 'partner', 'current', 0, 30, reply.known).map(part => [part.id, part.raw]), [[B, 5n]]);
  assert.deepEqual(countryParts(imports, 'imports', 'partner', 'total', 0, 47, reply.known).map(part => [part.id, part.raw]), [[B, 5n], [C, 0n]]);
  assert.deepEqual(countryParts(imports, 'imports', 'resource', 'total', 0, 47, reply.known).map(part => [part.id, part.raw]), [['2', 5n], ['9', 0n]]);
  assert.equal(countryTotal(exports, 'total', 0, 71, reply.known), 30n);
  assert.equal(countryTotal(exports, 'average', 0, 71, reply.known), 10n);
  const [buyer] = countryParts(exports, 'exports', 'partner', 'current', 0, 60, reply.known);
  assert.equal(partnerFact(buyer, 'exports', 60)?.tag, 'BBB');
  assert.equal(partnerFact(countryParts(countryTrade(trade, B, null).imports, 'imports', 'partner', 'current', 0, 60, reply.known)
    .find(part => part.id === A)!, 'imports', 60)?.tag, 'AAZ');
});

test('unrecorded days are unknown, never no trade', () => {
  const { imports } = countryTrade(trade, A, null);
  assert.equal(countryTotal(imports, 'current', 0, 100, reply.known), null);
  assert.deepEqual(countryParts(imports, 'imports', 'partner', 'current', 0, 100, reply.known), []);
  assert.equal(countryTotal(imports, 'total', 96, 120, reply.known), null);
  assert.notEqual(countryTotal(imports, 'total', 0, 120, reply.known), null, 'a time partly recorded counts its recorded days');
});

test('each resource’s deals draw as one band, exports above the axis and imports mirrored below', () => {
  assert.deepEqual(bandGroups([2, 2, 9, 9, 9, 2]), [[0, 1], [2, 4], [5, 5]]);
  assert.deepEqual(bandGroups([]), []);
  const piece = { hours: [0, 1], bands: [{ low: [0, 0], high: [1, 2] }, { low: [1, 2], high: [3, 3] }, { low: [3, 3], high: [4, 6] }] };
  assert.deepEqual(mergedBands(piece, [[0, 1], [2, 2]]).bands, [{ low: [0, 0], high: [3, 3] }, { low: [3, 3], high: [4, 6] }]);
  assert.deepEqual(mirrored(mergedBands(piece, [[0, 2]])).bands, [{ low: [-4, -6], high: [-0, -0] }]);
  assert.deepEqual(divergingExtent([0, 10], [0, 4]), [-4, 10]);
  assert.deepEqual(divergingExtent(null, [0, 4]), [-4, 0]);
  assert.equal(divergingExtent(null, null), null);
});

test('5,000 trading partners group in one pass over the deals', () => {
  const imported = (count: number) => {
    const big: TradeFlowsReply = { ...reply, countries: Array.from({ length: count + 1 }, (_, index) => `T${String(index).padStart(5, '0')}`),
      countryIdentities: Array.from({ length: count + 1 }, (_, index) => ({ tag: `T${String(index).padStart(5, '0')}`, runs: [{ firstHour: 0, lastHour: 95, slot: index + 10, countryId: index + 10 }] })),
      flows: Array.from({ length: count }, (_, index) => ({ seller: index + 1, buyer: 0, sellerIdentity: `[${index + 11},${index + 11}]`, buyerIdentity: '[10,10]',
        resource: index % 2 ? 2 : 9, firstHours: [0], lastHours: [95], deliveredRaw: [String(index + 1)],
        sellerTags: [index + 1], buyerTags: [0], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] })) };
    return { imports: countryTrade(decodeTrade(big), '[10,10]', null).imports, known: big.known };
  };
  assertLinearWork(count => {
    const { imports, known } = imported(count);
    let partners: ReturnType<typeof countryParts> = [], total: bigint | null = null;
    const work = measureWork(imports, flows => { partners = countryParts(flows, 'imports', 'partner', 'current', 0, 50, known); total = countryTotal(flows, 'current', 0, 50, known); });
    assert.equal(partners.length, count);
    assert.equal(total, BigInt(count * (count + 1) / 2));
    return work;
  }, 'grouping deals by partner and totalling them');
  assertLinearWork(count => {
    let groups: [number, number][] = [];
    const work = measureWork(imported(count).imports.map(flow => flow.resource.id), keys => { groups = bandGroups(keys); });
    assert.equal(groups.length, 2, 'two resources draw two bands');
    return work;
  }, 'grouping band keys');
});

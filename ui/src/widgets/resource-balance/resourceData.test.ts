import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey, valuesOf } from '../../charts/live.ts';
import type { ResourceBalanceReply } from '../../generated/dashboard';
import { POOL_CELLS, balanceOf, barStructure, breakdownRows, campaignScale, dealAccounting, dealAt, dealsOf, momentScale, orderResources, partWidth, poolsAt, poolsMissing, poolsOf, resourceName, resourcePartGroups, resourceParts, validateResourceBalance, validateResourceSizes } from './resourceData.ts';
import type { ResourcePools } from './resourceData';

const pools = (id: number, produced: string, production = '-300000', subjects = '0'): ResourcePools => ({
  id, producedRaw: produced, subjectsRaw: subjects, importedRaw: '0', toExportRaw: '100000', exportedRaw: '100000',
  availableRaw: produced, afterProjectsRaw: produced, productionRaw: production,
});

const cells = (...held: ResourcePools[]) => valuesOf(new Map(held.flatMap(item => POOL_CELLS.map(([column, field]) =>
  [liveKey(String(item.id), column), Number(item[field])] as const))), 'No live row has this key');

const reply = (): ResourceBalanceReply => ({
  tag: 'AAA', decimals: 5, warnings: [], poolsMissing: null, dealsMissing: null,
  resources: [
    { id: 1, token: 'oil', ordinal: 1, name: 'Oil', icon: null },
    { id: 2, token: 'steel', ordinal: 1, name: null, icon: null },
    { id: 3, token: 'steel', ordinal: 2, name: null, icon: null },
  ],
  pools: [1, 2],
  shown: [2, 1, 3],
  deals: [
    { deal: '6', resource: 2, direction: 'export', partner: 'BBB', factories: 1, convoysAssigned: 2, convoysRequired: 3 },
    { deal: '7', resource: 2, direction: 'export', partner: 'CCC', factories: 1, convoysAssigned: 0, convoysRequired: 0 },
  ],
});

test('a balance splits the recorded pools into supply, demand and the game net', () => {
  const balance = balanceOf(pools(1, '200000', '-300000', '-50000'));
  assert.deepEqual([balance.supply, balance.demand, balance.net, balance.toOverlord], [200000n, 300000n + 50000n + 100000n, -100000n, 50000n]);
});

test('pools are read from their live cells, exactly, and a missing cell leaves the resource unknown with its reason', () => {
  const values = cells(pools(1, '200000'), pools(2, '900000', '-100000'));
  assert.deepEqual(poolsAt(values, 2), pools(2, '900000', '-100000'));
  assert.equal(poolsAt(values, 3), null);
  assert.equal(poolsMissing(values, 3), 'No live row has this key');
  assert.deepEqual(poolsOf(reply(), values).map(item => item.id), [1, 2]);
  const gone = valuesOf(new Map([[liveKey('1', 'produced'), 5]]), 'Not recorded at this hour');
  assert.equal(poolsAt(gone, 1), null, 'one pool unknown is the resource unknown, never zero');
});

test('the bars keep the product order; a render follows which resources show and in which order, the amounts are painted', () => {
  const data = validateResourceBalance(reply());
  assert.deepEqual(orderResources(data).map(item => item.id), [2, 1, 3]);
  assert.equal(resourceName(data.resources[2]), 'Resource name unavailable (2)', 'a missing name is said, never the token');
  assert.equal(barStructure({ ...data, deals: [] }), barStructure(data), 'deals are not the bars');
  assert.notEqual(barStructure({ ...data, shown: [1, 2, 3] }), barStructure(data), 'a new order renders');
  assert.notEqual(barStructure({ ...data, pools: [1] }), barStructure(data), 'a row leaving renders');
});

test('the campaign scale is the longest bar over the span, the moment scale the longest shown', () => {
  const sizes = validateResourceSizes({ tag: 'AAA', sizes: [{ id: 1, longestRaw: '5' }, { id: 2, longestRaw: '1200000' }, { id: 3, longestRaw: null }], missing: [], warnings: [] });
  assert.equal(campaignScale(sizes), 1200000n);
  assert.equal(momentScale([pools(1, '200000'), pools(2, '900000', '-100000')]), 900000n);
  assert.equal(partWidth(-600000n, 1200000n), '50%');
});

test('a breakdown parts the balance as the game adds it: supply, needs, and the rounding left beside the net', () => {
  const balance = balanceOf({ ...pools(1, '200000', '-300000', '-50000'), afterProjectsRaw: '150000' });
  const parts = resourceParts(balance);
  assert.deepEqual(parts.map(part => part.id), ['produced', 'subjects', 'imported', 'projects', 'production', 'overlord', 'exported', 'unidentified']);
  assert.deepEqual(parts.map(part => part.raw), [200000n, 0n, 0n, -50000n, -300000n, -50000n, -100000n, balance.net - (balance.supply - balance.demand)]);
  const [supply, demand] = resourcePartGroups(balance);
  assert.equal(supply.total, balance.supply);
  assert.deepEqual(demand.values.map(part => part.raw), [50000n, 300000n, 50000n, 100000n], 'needs are positive in their group');
  assert.equal(demand.total, balance.demand);
});

const delivered = valuesOf(new Map([[liveKey('6', 'delivered'), 100000], [liveKey('7', 'delivered'), -250000]]), 'Not recorded at this hour');

test('a deal list sums every deal, those off screen too, and scales bars by the widest magnitude; one not known leaves the total unknown', () => {
  assert.deepEqual(dealAccounting(reply().deals.map(deal => dealAt(deal, delivered))), { total: -150000n, widest: 250000n });
  const partly = valuesOf(new Map([[liveKey('6', 'delivered'), 100000]]), 'Not recorded at this hour');
  assert.deepEqual(dealAccounting(reply().deals.map(deal => dealAt(deal, partly))), { total: null, widest: 100000n });
  assert.equal(dealAt(reply().deals[1], partly).deliveredRaw, null, 'a delivery not known is null, never zero');
});

test('deals unknown at the hour are said so, never shown as none', () => {
  const data = reply();
  assert.deepEqual(dealsOf(data, 2, delivered)?.exports.map(deal => deal.deliveredRaw), ['100000', '-250000']);
  const rows = breakdownRows(balanceOf(pools(2, '900000', '-100000')), dealsOf({ ...data, dealsMissing: 'not recorded' }, 2, delivered), 5, tag => ({ name: tag }) as never);
  assert.ok(rows.some(row => row.label === 'Trade partners not recorded at this hour'));
});

test('a malformed reply is refused', () => {
  assert.throws(() => validateResourceBalance({ ...reply(), pools: [9] }), /malformed/);
  assert.throws(() => validateResourceBalance({ ...reply(), shown: [9] }), /malformed/);
  assert.throws(() => validateResourceBalance({ ...reply(), resources: [...reply().resources].reverse() }), /malformed/);
});

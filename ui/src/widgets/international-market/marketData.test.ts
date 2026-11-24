import test from 'node:test';
import assert from 'node:assert/strict';
import { validateMarketReply } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import { contractRows, marketDetail, offerRows } from './marketData.ts';
import { internationalMarketProjection } from './internationalMarketProjection.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { liveKey, valuesOf } from '../../charts/live.ts';

const item = (variant: string, amountRaw: string) => ({ variant, name: null, amountRaw });
const raw = { branchId: '1', purchasesMissing: null, unattributed: 1, refused: null, warnings: [],
  variants: [{ id: '5', hour: 7, known: true, token: 'infantry_equipment', name: 'Rifle', typeName: 'Infantry equipment', nameUnknown: null, icon: 'a', iconUnknown: null }],
  icons: [{ key: 'a', image: 'data:rifle' }],
  offers: [
    { country: 44, tag: 'FRA', inventory: [item('5', '300000'), item('6', '100000')], prices: [{ variant: '6', name: null, level: 2 }], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false },
    { country: 45, tag: null, inventory: [item('5', '1')], prices: [], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false },
  ],
  contracts: [
    { key: '1', id: '51', seller: 'FRA', buyer: 'GER', request: false, speed: 1, equipment: [item('5', '100000'), item('6', '1')], prices: [], levels: [], subsidies: [], purchase: true },
    { key: '2', id: '52', seller: 'FRA', buyer: null, request: true, speed: 0, equipment: [item('5', '1')], prices: [], levels: [{ variant: '5', name: null, level: 0 }], subsidies: [], purchase: false },
    { key: '3', id: '53', seller: 'ITA', buyer: 'ENG', request: false, speed: 0, equipment: [], prices: [], levels: [], subsidies: [], purchase: false },
    { key: '4', id: '54', seller: 'ITA', buyer: 'ENG', request: true, speed: 0, equipment: [item('6', '200000')], prices: [], levels: [{ variant: '5', name: null, level: 0 }, { variant: '6', name: null, level: 2 }], subsidies: [], purchase: false },
  ],
  agreements: null };
const reply: MarketAt = { ...validateMarketReply(raw), hour: 7 };

test('each tab lists the product\'s own rows of its kind, each variant in native order with its price level', () => {
  const contracts = contractRows(reply, 'contracts');
  assert.deepEqual(contracts.map(row => row.key), ['1', '3']);
  assert.deepEqual(contracts[0].equipment.map(entry => [entry.name, entry.icon, entry.amount]), [['Rifle', 'data:rifle', 100000n], [null, null, 1n]]);
  assert.deepEqual(contractRows(reply, 'requests')[0].levels, [{ variant: '5', level: 0 }]);
  const rows = offerRows(reply);
  assert.deepEqual(rows.map(row => [row.key, row.name, row.amount, row.level]), [['44:5', 'Rifle', 300000n, null], ['44:6', null, 100000n, 2], ['45:5', 'Rifle', 1n, null]]);
  assert.equal(rows[1].missing, 'The recording does not name this design at this hour.');
});

test('the product\'s count of rows it could not attribute and its refused membership are shown as not recorded, never as none', () => {
  const view = (panel: unknown) => createBoundedReadView({ hour: 7, panels: new Map([['main', panel]]) });
  const chosen = internationalMarketProjection.project(view({ ...raw, contracts: [] }), { view: 'requests' });
  assert.equal(chosen.emptyKind, 'unavailable', 'a request with an unnamed buyer may be the chosen country\'s');
  const refused = internationalMarketProjection.project(view({ ...raw, offers: null, contracts: null, refused: 'Membership of this group is not recorded at this hour' }), { view: 'contracts' });
  assert.deepEqual([refused.empty, refused.emptyDetail], ['Not recorded', 'Membership of this group is not recorded at this hour']);
});


test('contract cells show the live figures exactly as recorded and say why one is missing', () => {
  const collectedRaw = (1n << 60n) + 7n;
  const values = new Map<string, number>([[liveKey('1', 'assigned'), 2], [liveKey('1', 'convoys'), 3], [liveKey('1', 'deliveries'), 4], [liveKey('1', 'collected'), Number(collectedRaw)]]);
  const live = { ...valuesOf(values, 'Convoy progress was not recorded at this hour'), raw: (key: string) => (key === liveKey('1', 'collected') ? collectedRaw : undefined) };
  const model = internationalMarketProjection.project(createBoundedReadView({ hour: 7, panels: new Map([['main', raw]]), live: new Map([['main', live]]) }), {
    view: 'contracts',
  });
  const cells = model.rows.find(row => row.key === '1')!.cells;
  assert.deepEqual(cells.collected!.value, { kind: 'known', value: collectedRaw }, 'a collected payment beyond 2^53 stays exact');
  assert.deepEqual([cells.convoys!.value, cells.convoys!.secondaryValue], [{ kind: 'known', value: 2 }, { kind: 'known', value: 3 }]);
  assert.deepEqual(cells.deliveries!.value, { kind: 'known', value: 4 });
  const unread = model.rows.find(row => row.key === '3')!.cells;
  assert.deepEqual(unread.collected!.value, { kind: 'notRecorded', reason: 'Convoy progress was not recorded at this hour' });
});

test('each amount, price and level belongs to its own variant: reordered levels never swap prices between variants, and several equipment are listed one by one', () => {
  const view = createBoundedReadView({ hour: 7, panels: new Map([['main', raw]]) });
  const requests = internationalMarketProjection.project(view, { view: 'requests' });
  assert.deepEqual(requests.rows.find(row => row.key === '4')!.cells.level!.value, { kind: 'known', value: 'High' }, 'variant 6 is High though variant 5 is listed first');
  const contracts = internationalMarketProjection.project(view, { view: 'contracts' });
  const several = contracts.rows.find(row => row.key === '1')!;
  assert.equal(several.cells.price!.value.kind, 'notApplicable', 'two equipment never show one price');
  const extra = (typeof several.tooltipExtra === 'function' ? several.tooltipExtra() : several.tooltipExtra) ?? [];
  assert.deepEqual(extra.filter(entry => entry.lookupKey).map(entry => entry.lookupKey), ['5', '6']);
});

test('a row opens its own detail from its full recorded key: an inventory click marks its own equipment, a contract lists each equipment with its own price and level', () => {
  const offer = marketDetail(raw, { section: 'offers', row: '44', variant: '6' });
  assert.equal(offer.missing, null);
  const listed = offer.sections.find(section => section.title === 'Listed inventory')!.lines;
  assert.deepEqual(listed.map(line => [line.label, line.marked]), [['Rifle', false], ['Equipment name unavailable', true]], 'two equipment rows of one seller open different details');
  assert.match(listed[1]!.value, /High/);
  const request = marketDetail(raw, { section: 'contracts', row: '4', variant: null }).sections.find(section => section.title === 'Equipment')!.lines;
  assert.match(request[0]!.value, /High/, 'the level of its own variant, though another variant is listed first');
  assert.equal(marketDetail(raw, { section: 'contracts', row: '99', variant: null }).missing, 'This request or contract is not on the market at this date.');
  assert.equal(marketDetail({ ...raw, offers: null }, { section: 'offers', row: '44', variant: '5' }).missing, 'Market inventory was not recorded at this date.');
  const twin = { ...raw, contracts: [...raw.contracts, { key: '5', id: '54', seller: 'GER', buyer: 'ENG', request: true, speed: 9, equipment: [item('5', '1')], prices: [], levels: [], subsidies: [], purchase: false }] };
  const seller = (row: string) => marketDetail(twin, { section: 'contracts', row, variant: null }).sections[0]!.lines.find(line => line.label === 'Seller')!.value;
  assert.deepEqual([seller('4'), seller('5')], ['ITA', 'GER'], 'two requests that differ only in the seller open their own terms');
  const unnamed = marketDetail(raw, { section: 'offers', row: '45', variant: '5' });
  assert.equal(unnamed.missing, null, 'an offer whose seller has no country at the hour still opens from its raw seller');
  assert.equal(unnamed.sections[0]!.lines.length, 1);
});

test('a contract of several equipment labels each variant\'s agreed price with its missing state, and an empty contract is not several equipment', () => {
  const view = createBoundedReadView({ hour: 7, panels: new Map([['main', raw]]) });
  const contracts = internationalMarketProjection.project(view, { view: 'contracts' });
  const several = contracts.rows.find(row => row.key === '1')!;
  const extra = (typeof several.tooltipExtra === 'function' ? several.tooltipExtra() : several.tooltipExtra) ?? [];
  for (const entry of extra.filter(line => line.lookupKey)) assert.match(JSON.stringify(entry.value), /Agreed price not recorded/, `variant ${entry.lookupKey}`);
  const lines = marketDetail(raw, { section: 'contracts', row: '1', variant: null }).sections.find(section => section.title === 'Equipment')!.lines;
  assert.deepEqual(lines.map(line => /Agreed price not recorded/.test(line.value)), [true, true]);
  const empty = contracts.rows.find(row => row.key === '3')!.cells.price!.value;
  assert.ok(empty.kind === 'notApplicable' && !/Several/.test(empty.reason), `an empty contract's price: ${JSON.stringify(empty)}`);
});

test('the market offers no option its detail window does not read', async () => {
  const { definition } = await import('./definition.ts');
  assert.deepEqual(definition.options.map(field => field.key), ['view']);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { rawAmount, validateMarketReply, marketRequest } from './marketData.ts';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
test('market and lend-lease preserves complete fixed point values and fuel scales', () => {
  assert.equal(rawAmount('9007199254740993'), '90071992547.40993');
  assert.equal(rawAmount('-1'), '-0.00001');
  assert.equal(rawAmount('32769', 32768n), '1.00003');
  assert.equal(rawAmount('10001', 1000n), '10.001');
});
test('market catalogue card is a switchable Trade flows view', () => {
  assert.equal(definition.kind, 'trade-flows');
  assert.equal(resolveOptions(OPTIONS, {}).view, 'resources');
  const card = definition.variants.find(card => card.key === 'view=market');
  assert.equal(card?.initial.title, 'Market & lend-lease');
  assert.equal(card?.initial.options?.view, 'market');
  assert.equal(resolveOptions(OPTIONS, card?.initial.options).view, 'market');
  assert.equal(resolveOptions(OPTIONS, { ...card?.initial.options, view: 'resources' }).view, 'resources');
});

test('market unavailable collections remain unknown and malformed nested values refuse', () => {
  const reply = { branchId: '1', hour: 7, offers: null, contracts: [], agreements: [], deliveries: [], warnings: ['writer unavailable'] };
  assert.equal(validateMarketReply(reply).offers, null);
  assert.throws(() => validateMarketReply({ ...reply, offers: [{ country: 5000, tag: 'AA', inventory: [{ variant: '1', name: null, amountRaw: 1 }], prices: [], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false }] }));
});
test('market requests preserve source, revision, date and unlimited country demand', () => {
  const track = { source: { id: 'source', revision: 'revision' }, branch: { id: '4' } };
  const tags = Array.from({ length: 5000 }, (_, id) => `X${id}`);
  const request = marketRequest(track as Parameters<typeof marketRequest>[0], 7.8, tags);
  assert.equal(request.product, 'market-lendlease');
  assert.equal((request.params as { hour: number }).hour, 7);
  assert.equal((request.params as { tags: string[] }).tags.length, 5000);
  assert.equal(request.revision, 'revision');
});

test('Trade flows market presentation keeps terms, schedules and terminal deliveries at the supplied hour', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeMarketView.tsx');
    const item = { variant: '31', name: 'Mod equipment', amountRaw: '9007199254740993' };
    const data = validateMarketReply({ branchId: '1', hour: 77, warnings: [],
      offers: [{ country: 5000, tag: 'AAA', inventory: [item], prices: [{ variant: '31', name: 'Mod equipment', level: 2 }], subsidies: [], subsidyTotalRaw: '17', autoSendMarketAccess: true }],
      contracts: [{ id: '17', seller: 'AAA', buyer: 'BBB', request: false, speed: 3, equipment: [item], prices: [{ ...item, amountRaw: '12345' }], levels: [], subsidies: [] }],
      agreements: [{ id: '18', giver: 'AAA', receiver: 'BBB', revision: '3', activeSince: 1, lastDelivery: 77,
        monthly: [item], percentage: [], once: [], delivered: [item], sunk: [], fuelDailyRaw: '32769', fuelPercentageRaw: '10001', fuelSentRaw: '17', fuelSunkRaw: '0', lastFuelDeliveredRaw: '17' }],
      deliveries: [{ id: '1', occurrence: '7', agreement: '18', hour: 77, kind: 6, giver: 'AAA', receiver: 'BBB', item: { ...item, amountRaw: '0' }, lostRaw: '0', totalRaw: '0', lostTotalRaw: '0' }] });
    const render = (selected?: string) => renderToStaticMarkup(createElement(View, { data, selected }));
    const html = render();
    for (const title of ['International market', 'Lend-lease', 'Listed inventory', 'Asking price levels', 'Agreed prices (CIC)', 'Monthly', 'Once', 'Delivered', 'Agreement ended']) assert.ok(html.includes(title), title);
    assert.match(html, /90071992547[.]40993/);
    assert.match(html, /Mod equipment/);
    assert.match(render('agreement:18'), /<details open=""/);
    assert.doesNotMatch(render('agreement:18'), /<summary[^>]*>.*Contract<\/summary>/);
    const unknown = renderToStaticMarkup(createElement(View, { data: { ...data, offers: null, contracts: null, agreements: null, deliveries: null } }));
    assert.match(unknown, /not recorded/);
    assert.doesNotMatch(unknown, /No active agreements/);
    const partial = validateMarketReply({ ...data, deliveries: [data.deliveries![0], {
      ...data.deliveries![0], id: '2', agreement: '19', kind: 34,
      item: { variant: '0', name: 'Delivery amount unavailable: cumulative baseline uncertified', amountRaw: '0' },
    }] });
    const missingAmount = renderToStaticMarkup(createElement(View, { data: partial }));
    assert.match(missingAmount, /Agreement ended/);
    assert.match(missingAmount, /Delivery amount unavailable: cumulative baseline uncertified/);
    assert.doesNotMatch(missingAmount, /No deliveries observed/);
    assert.doesNotMatch(missingAmount, /Equipment name unavailable: 0/);
  } finally { await vite.close(); }
});

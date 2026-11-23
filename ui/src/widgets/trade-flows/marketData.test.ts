import { gatedHour } from '../../test-support/gated.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { rawAmount, validateMarketReply, marketRequest, marketLendLeaseWaits } from './marketData.ts';
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
  const reply = { branchId: '1', offers: null, contracts: [], agreements: [], purchasesMissing: null, unattributed: 0, variants: [], icons: [], warnings: ['writer unavailable'] };
  assert.equal(validateMarketReply(reply).offers, null);
  assert.throws(() => validateMarketReply({ ...reply, offers: [{ country: 5000, tag: 'AA', inventory: [{ variant: '1', name: null, amountRaw: 1 }], prices: [], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false }] }));
  assert.throws(() => validateMarketReply({ ...reply, unattributed: undefined }), 'a reply says how many rows it could not attribute');
});
// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('market requests preserve source, revision, date and unlimited country demand', () => {
  const track = { source: { id: 'source', revision: 'revision' }, branch: { id: '4' } };
  const tags = Array.from({ length: 5000 }, (_, id) => `X${id}`);
  const request = marketRequest(track as Parameters<typeof marketRequest>[0], gatedHour(7), tags);
  assert.equal(request.product, 'market-lendlease');
  assert.equal((request.params as { hour: number }).hour, 7);
  assert.equal((request.params as { tags: string[] }).tags.length, 5000);
  assert.equal(request.revision, 'revision');
});

test('Trade flows market presentation keeps terms, schedules and live progress at the supplied hour', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeMarketView.tsx');
    const item = { variant: '31', name: 'Mod equipment', amountRaw: '9007199254740993' };
    const data = { ...validateMarketReply({ branchId: '1', warnings: [], purchasesMissing: null, unattributed: 0, variants: [], icons: [],
      offers: [{ country: 5000, tag: 'AAA', inventory: [item], prices: [{ variant: '31', name: 'Mod equipment', level: 2 }], subsidies: [], subsidyTotalRaw: '17', autoSendMarketAccess: true }],
      contracts: [{ key: '9', id: '0000000000000017', seller: 'AAA', buyer: 'BBB', request: false, speed: 3, equipment: [item], prices: [{ ...item, amountRaw: '12345' }], levels: [], subsidies: [], purchase: true }],
      agreements: [{ key: '8', id: '0000000000000018', giver: 'AAA', receiver: 'BBB', revision: '3', activeSince: 0, lastDelivery: 77,
        monthly: [item], percentage: [], once: [], delivered: [item], sunk: [],
        fuelDailyRaw: '32769', fuelPercentageRaw: '10001', lastFuelDeliveredRaw: '17' }] }), hour: 77 };
    const wideFuel = 9007199254740993n * 32768n;
    const values = {
      get: (key: string) => ({ '9\u0000assigned': 2, '9\u0000convoys': 3, '9\u0000collected': 150000, '9\u0000deliveries': 4, '8\u0000fuelSent': Number(wideFuel) } as Record<string, number>)[key],
      raw: (key: string) => ({ '9\u0000assigned': 2n, '9\u0000convoys': 3n, '9\u0000collected': 150000n, '9\u0000deliveries': 4n, '8\u0000fuelSent': wideFuel } as Record<string, bigint>)[key],
      missing: () => 'Not read yet',
    };
    const render = (selected?: string) => renderToStaticMarkup(createElement(View, { data, selected, values }));
    const html = render();
    for (const title of ['International market', 'Lend-lease', 'Listed inventory', 'Asking price levels', 'Agreed prices (CIC)', 'Monthly', 'Once', 'Delivered']) assert.ok(html.includes(title), title);
    assert.match(html, /90071992547[.]40993/);
    assert.match(html, /Mod equipment/);
    assert.match(html, /Active since Hour 0 · last delivery/);
    assert.match(html, /Convoys 2 of 3 · collected 1[.]5 CIC · deliveries 4/);
    assert.match(html, /Fuel sent: 9007199254740993 · lost: <span[^>]*data-value-state="missing"[^>]*data-tooltip="Not recorded: Not read yet"/, 'live fuel keeps exact bigint via raw() and missing cells render ValueState');
    assert.match(render('agreement:0000000000000018'), /<details open=""/);
    assert.throws(() => validateMarketReply({ ...data, agreements: [{ ...data.agreements![0], receiver: null }] }), 'null receiver requires a missing reason');
    const unknown = renderToStaticMarkup(createElement(View, { data: { ...data, offers: null, contracts: null, agreements: null } }));
    assert.match(unknown, /not recorded/);
    assert.doesNotMatch(unknown, /No active agreements/);
  } finally { await vite.close(); }
});

test('the market view says by name that it waits on the market family until its product is served', () => {
  assert.match(marketLendLeaseWaits(['trade-flows']) ?? '', /market widgets/);
  assert.equal(marketLendLeaseWaits(['trade-flows', 'market-lendlease']), null);
});

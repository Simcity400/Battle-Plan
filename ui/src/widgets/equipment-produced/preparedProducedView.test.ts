import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { formatRaw, producedAt, producedModel, producedRequest, producedTooltips, rankProduced } from './producedData.ts';
import { formatProductionAmount, productionRaceEntries } from './preparedProducedData.ts';
import type { ProductionSelection } from './preparedProducedData';
import { historyProductionParity, preparedProductionParity } from './productionParity.ts';

const page = (total: string | null = '876500'): ProductionSelection => ({ schema: 1, branchId: '1', firstHour: 10, lastHour: 23, known: true, observation: 0,
  keys: [{ key: 'one', name: 'One', typeKey: null, domain: 'land', names: [] }, { key: 'two', name: 'Two', typeKey: null, domain: 'land', names: [] }],
  types: [], totalCountries: 1, offset: '0', valueMin: '0', valueMax: total ?? '0', warnings: [],
  countries: [{ tag: 'A', slot: 17, countryId: 42, segment: '3', total, equipmentTotal: total,
    readHour: 10, observation: 0, values: [{ key: 'one', amount: total, equipmentAmount: total }, { key: 'two', amount: '0', equipmentAmount: '0' }] }] });

test('prepared Units and IC/day retain selected counts, native continuity and main formatting without counting twice', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { productionEndpointReply, productionEndpointTooltips } = await vite.ssrLoadModule('/ui/src/widgets/equipment-produced/preparedProducedView.ts') as typeof import('./preparedProducedView');
    for (const count of ['units', 'ic'] as const) {
      const decimals = count === 'ic' ? 5 : 0;
      const params = producedRequest('1', { tags: ['A'] }, 'day', 'type', [], false, count);
      const current = page();
      const model = producedModel(productionEndpointReply(current, params));
      const at = producedAt(model, 'A', 12)!;
      assert.equal(at.total, 876500, 'the host already measured the day, so the client does no subtraction');
      assert.equal(producedAt(model, 'A', 9), null, 'the endpoint invents no earlier history');
      assert.equal(producedAt(model, 'A', 24), null, 'the endpoint never carries beyond its recorded range');
      const entries = productionRaceEntries(current, () => [], false, 12, decimals);
      assert.equal(entries[0].value, 876500 / 10 ** decimals);
      assert.equal(entries[0].label, formatRaw(876500, decimals));
      assert.equal(entries[0].entityKey, '17:42');
      assert.equal(entries[0].stretch, '17:42:3:0');
      const expected = [...producedTooltips(model, 'A', 'Country A', at, 12, '1 Jan 1', [], 'What is counted', decimals)];
      const actual = productionEndpointTooltips(current, params, 'A', 12, 'Country A', 'What is counted')!;
      assert.deepEqual(actual.map(item => item.rows), expected.map(item => item.rows), 'the redesigned tooltip uses the same exact units/rate rule');
      const ranked = rankProduced(model, ['A'], 12, 1);
      assert.deepEqual(preparedProductionParity(current, true), historyProductionParity(model, ranked, true));
      const zero = page('0');
      assert.equal(productionRaceEntries(zero, () => [], false, 12, decimals)[0].label, '0', 'manual recorded zero survives');
      assert.deepEqual(productionRaceEntries(page(null), () => [], false, 12, decimals), [], 'an unknown total is never drawn as zero');
    }
    for (const raw of [0, 1, -1, 875, -875, 999999, 1000000, 9007199254740991]) {
      for (const decimals of [0, 5]) assert.equal(formatProductionAmount(String(raw), decimals), formatRaw(raw, decimals));
    }
  } finally { await vite.close(); }
});

test('prepared canonical rows preserve unknown equipment subtotals and ranking while exact operands survive geometry rounding', () => {
  const current = page('9007199254740993');
  current.countries[0].values[0] = { key: 'one', amount: null, equipmentAmount: '9007199254740992' };
  const model = preparedProductionParity(current, true);
  assert.deepEqual(model.rows, [{ key: 'A', index: 0, tag: 'A', total: '9007199254740993',
    values: [{ key: 'one', amount: null, equipmentAmount: '9007199254740992' }] }]);
  assert.equal(productionRaceEntries(current, () => [], false, 12)[0].label, '9,007,199,254,740,993');
  assert.equal(formatProductionAmount(null, 5), 'Not known');
});

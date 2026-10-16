import assert from 'node:assert/strict';
import { test } from 'node:test';
import { exactProductionAmount, productionBandAmount, productionBands, productionKeyLabel, productionRaceEntries, validateProductionSelection } from './preparedProducedData.ts';
import type { ProductionCountry, ProductionSelection, ProductionSelectionRequest } from './preparedProducedData';
import { queryProductionSelection } from './usePreparedProduction.ts';
import type { DashboardSource } from '../../dashboard/client';

const request: ProductionSelectionRequest = { hour: 12, offset: '0', limit: 5000, ranked: true,
  selection: { branchId: '1', tags: [], top: 5000, measure: 'total', count: 'units', grouping: 'type', included: ['mod_one', 'mod_one'], refits: false } };
const key = (key: string, name: string | null) => ({ key, name, typeKey: null, domain: 'land' as const, names: [] });
const country = (tag: string, total: string | null): ProductionCountry => ({ tag, slot: 1, countryId: 2, segment: '1', readHour: 0, observation: 0,
  total, equipmentTotal: total, values: [{ key: 'one', amount: total, equipmentAmount: total }, { key: 'two', amount: '0', equipmentAmount: '0' }] });
const page = (): ProductionSelection => ({ schema: 1, branchId: '1', firstHour: 0, lastHour: 23, known: true,
  keys: [key('one', ''), key('two', 'Two')], types: [], totalCountries: 2,
  offset: '0', valueMin: '0', valueMax: '9007199254740993',
  countries: [country('B', '9007199254740993'), country('A', '9007199254740992')], warnings: [] });

test('prepared pages keep exact host ordering and decimal values above safe53', () => {
  const data = validateProductionSelection(page(), request);
  const bands = productionBands(data, 'type', 12, ['first', 'second'], 'muted');
  const entries = productionRaceEntries(data, () => bands, false, 12);
  assert.deepEqual(entries.map(entry => entry.tag), ['B', 'A']);
  assert.equal(entries[0].label, exactProductionAmount('9007199254740993'));
  assert.notEqual(entries[0].label, entries[1].label);
  assert.equal(entries[0].value, entries[1].value); // Only geometry rounds.
  assert.equal(entries[0].hour, 12);
  assert.equal(bands[0].label, ''); // Authored empty names stay empty.
});

test('folded groups aggregate signed integers before geometry and preserve missing subtotals', () => {
  const row = country('A', '1');
  row.values[0].amount = '9007199254740993'; row.values[1].amount = '-9007199254740992';
  const band = { id: '', label: 'Other', color: 'muted', keys: ['one', 'two'] };
  assert.equal(productionBandAmount(row, band), '1');
  row.values[0].amount = null;
  assert.equal(productionBandAmount(row, band), null);
  assert.equal(productionBandAmount(row, band, true), '1');
  assert.equal(exactProductionAmount(null), 'Not known');
  assert.equal(exactProductionAmount('0'), '0');
});

test('split bars retain signed parts and compute unknown residuals exactly before plotting', () => {
  const data = page();
  data.countries = [country('A', '1')];
  const bands = productionBands(data, 'type', 12, ['first', 'second'], 'muted');
  data.countries[0].values[0].amount = '-5'; data.countries[0].values[1].amount = '6';
  assert.deepEqual(productionRaceEntries(data, () => bands, true)[0].parts?.map(part => part.value), [-5, 6]);
  data.countries[0].total = '9007199254740993'; data.countries[0].values[0].amount = '9007199254740992';
  data.countries[0].values[1].amount = null;
  assert.equal(productionRaceEntries(data, () => bands, true)[0].parts?.at(-1)?.hatched, 1);
});

test('all 5000 country DTOs validate and malformed page identities refuse', () => {
  const data = page(); data.totalCountries = 5000;
  data.countries = Array.from({ length: 5000 }, (_, index) => ({ ...country(`M${index}`, String(index)), slot: index, countryId: index }));
  assert.equal(validateProductionSelection(data, request).countries.length, 5000);
  assert.throws(() => validateProductionSelection(data, { ...request, selection: { ...request.selection, branchId: '2' } }), /malformed/);
  assert.throws(() => validateProductionSelection(data, { ...request, limit: 4999 }), /malformed/);
  assert.throws(() => validateProductionSelection(data, { ...request, hour: 24 }), /malformed/);
  data.countries[0].total = '1e16';
  assert.throws(() => validateProductionSelection(data, request), /malformed/);
});

test('variant labels never consume a future read', () => {
  const item = { ...key('10', null), names: [{ firstHour: 20, name: 'Later', known: true, design: null }] };
  assert.equal(productionKeyLabel(item, 'variant', 12), 'Variant 10');
  assert.equal(productionKeyLabel(item, 'variant', 20), 'Later');
});

test('prepared transport receives exact page selection and ignores cancelled late replies', async () => {
  const source = { id: 'source', revision: 'revision' } as DashboardSource;
  const controller = new AbortController();
  let finish!: (value: unknown) => void;
  const pending = new Promise<unknown>(resolve => { finish = resolve; });
  const client = { preparedQuery: async <T>(input: DashboardSource, product: string, params: unknown, options: { signal: AbortSignal }) => {
    assert.equal(input, source); assert.equal(product, 'production-select'); assert.equal(params, request); assert.equal(options.signal, controller.signal);
    return await pending as T;
  } };
  const reply = queryProductionSelection(client, source, request, controller.signal);
  controller.abort(); finish(page());
  await assert.rejects(reply, { name: 'AbortError' });
});


test('pending preparation null bypasses the production DTO validator', async () => {
  const client = { preparedQuery: async <T>() => null as T };
  assert.equal(await queryProductionSelection(client, { id: 'source', revision: 'revision' } as DashboardSource,
    request, new AbortController().signal), null);
});

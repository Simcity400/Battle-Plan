import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions, sectionFields, setToken } from '../../dashboard/options.ts';
import { definition, normaliseAirOptions, OPTIONS } from './definition.ts';

test('saved race charts and loss causes migrate onto the butterfly and keep their filters', () => {
  for (const [saved, expected] of [[{ view: 'race' }, 'butterfly'], [{ view: 'lines' }, 'lines'], [{ view: 'race', show: 'breakdown' }, 'breakdown']] as const) {
    const options = { ...saved, regions: setToken('mod region'), causes: setToken('mod cause') };
    const migrated = definition.migrate(2, { configVersion: 2, options } as unknown as Parameters<typeof definition.migrate>[1]);
    assert.equal(migrated.configVersion, 3);
    assert.equal(migrated.options?.view, expected);
    assert.equal(migrated.options?.regions, options.regions);
    assert.deepEqual(normaliseAirOptions(migrated.options), migrated.options, 'normalisation is idempotent');
  }
  assert.deepEqual(normaliseAirOptions(), { view: 'butterfly' });
});

test('each view offers only its own settings', () => {
  for (const view of ['butterfly', 'lines', 'breakdown']) {
    const options = resolveOptions(OPTIONS, { view });
    const fields = sectionFields(OPTIONS, 'options', options).map(field => field.key);
    assert.equal(fields.includes('endLabels'), view === 'lines');
    assert.equal(sectionFields(OPTIONS, 'axes', options).some(field => field.key === 'yMax'), view === 'lines');
  }
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';

test('each view offers only its own settings', () => {
  for (const view of ['butterfly', 'lines', 'breakdown']) {
    const options = resolveOptions(OPTIONS, { view });
    const fields = sectionFields(OPTIONS, 'options', options).map(field => field.key);
    assert.equal(fields.includes('endLabels'), view === 'lines');
    assert.equal(sectionFields(OPTIONS, 'axes', options).some(field => field.key === 'yMax'), view === 'lines');
  }
});

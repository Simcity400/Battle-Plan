import assert from 'node:assert/strict';
import test from 'node:test';
import { OPTIONS } from './definition.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';

test('one body factor selector preserves saved factors and every hidden preference', () => {
  for (const show of ['efficiency', 'output']) {
    const saved = { show, factor: 'cap', line: 'saved-line', ended: 'hidden' };
    const options = resolveOptions(OPTIONS, saved);
    assert.deepEqual(options.factor, saved.factor);
    const fields = sectionFields(OPTIONS, 'options', options).map(field => field.key);
    assert.ok(fields.includes('line'));
    for (const key of ['factor', 'tableSettings', 'ended']) assert.ok(!fields.includes(key));
    const restored = resolveOptions(OPTIONS, { ...options, show: 'main' });
    for (const key of ['factor', 'line', 'ended'] as const) assert.equal(restored[key], saved[key]);
    assert.ok(!sectionFields(OPTIONS, 'options', restored).some(field => field.key === 'line'));
    for (const mode of ['campaign', 'range', 'follow', 'pin'] as const) {
      const mainFields = sectionFields(OPTIONS, 'options', restored, undefined, mode).map(field => field.key);
      assert.equal(mainFields.includes('tableSettings'), mode === 'follow' || mode === 'pin');
      assert.equal(mainFields.includes('ended'), mode === 'campaign' || mode === 'range');
      const drillFields = sectionFields(OPTIONS, 'options', options, undefined, mode).map(field => field.key);
      for (const key of ['tableSettings', 'ended', 'factor']) assert.ok(!drillFields.includes(key));
    }
  }
});

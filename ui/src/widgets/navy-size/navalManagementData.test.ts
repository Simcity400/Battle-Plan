import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fixedText, formatNavalAmount, validateNavalManagement } from './navalManagementData.ts';

const reply = { warnings: [], sections: [{ key: 'task:1', title: 'Fleet', lines: [
  { text: 'Ship 4 refit progress: ', amount: { row: '70', column: 'progress', raw: '123456', of: null, unit: 'IC' } },
  { text: '1. Ship 5 - hull HP ', amount: { row: '5', column: 'hull', raw: null, of: '2000000', unit: 'HP' } },
  { text: 'Repair priority: High', amount: null }] }] };

test('naval management amounts format fixed-5 values and preserve missing raw values as unavailable', () => {
  const parsed = validateNavalManagement(reply);
  assert.equal(formatNavalAmount(parsed.sections[0].lines[0].amount!), '1.23456 IC');
  assert.equal(formatNavalAmount(parsed.sections[0].lines[1].amount!), 'Unavailable / 20.00000');
  assert.equal(fixedText('-123'), '-0.00123');
  assert.throws(() => validateNavalManagement({ warnings: [], sections: [{ key: 'a', title: 'b', lines: ['text'] }] }));
});

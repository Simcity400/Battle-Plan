import assert from 'node:assert/strict';
import { test } from 'node:test';
import { valuesOf } from '../../charts/live.ts';
import { amountKey, amountText, fixedText, managementStructure, validateNavalManagement } from './navalManagementData.ts';

const reply = { warnings: [], sections: [{ key: 'task:1', title: 'Fleet', lines: [
  { text: 'Ship 4 refit progress: ', amount: { row: '70', column: 'progress', raw: '123456', of: null, unit: 'IC' } },
  { text: '1. Ship 5 - hull HP ', amount: { row: '5', column: 'hull', raw: null, of: '2000000', unit: 'HP' } },
  { text: 'Repair priority: High', amount: null }] }] };

test('a moving amount is painted from its live cell, else the reply’s value, never zero for missing', () => {
  const parsed = validateNavalManagement(reply);
  const progress = amountKey(parsed.sections[0].lines[0].amount!), hull = amountKey(parsed.sections[0].lines[1].amount!);
  assert.equal(amountText(valuesOf(new Map(), 'Not read yet'), progress, parsed), '1.23456 IC');
  assert.equal(amountText(valuesOf(new Map([[progress, 200000]]), 'Not read yet'), progress, parsed), '2.00000 IC');
  assert.equal(amountText(valuesOf(new Map(), 'Not read yet'), hull, parsed), 'Unavailable / 20.00000');
  assert.equal(fixedText('-123'), '-0.00123');
});

test('a value moving does not move the structure; a line appearing does', () => {
  const moved = { ...reply, sections: [{ ...reply.sections[0], lines: [{ ...reply.sections[0].lines[0], amount: { ...reply.sections[0].lines[0].amount!, raw: '1' } }, ...reply.sections[0].lines.slice(1)] }] };
  assert.equal(managementStructure(validateNavalManagement(moved)), managementStructure(validateNavalManagement(reply)));
  assert.throws(() => validateNavalManagement({ warnings: [], sections: [{ key: 'a', title: 'b', lines: ['text'] }] }));
});

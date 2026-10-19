import assert from 'node:assert/strict';
import test from 'node:test';
import { fixed5, validateWargoalsReply } from './warGoalsData.ts';
const fixture = () => ({ branchId: '1', hour: 80, tag: 'MOD', eligibility: [{ actor: 'MOD', recipient: 'TWO', justify: false as boolean | null, declare: null as boolean | null, justifyReason: 'Same faction', declareReason: 'Remaining gates unresolved' }], goals: [{ handle: '18446744073709551615', name: 'Custom goal', actor: 'MOD', recipient: 'TWO', available: true, expires: 100, states: [1, 2, 1], liberate: ['THREE', 'TWO'], puppets: ['FOUR'], terms: null }],
  justifications: [{ ordinal: 4, name: 'Custom goal', actor: 'MOD', recipient: 'TWO', states: [3, 1], total: '9000000', points: '1750000', daily: '100000', payUpfront: true, start: 50, completion: 120, threat: '0', paused: false, terms: null }], stateNames: [{ id: 1, name: 'Custom state', known: true }], warnings: ['Declare war eligibility refused (1)'] });
test('war-goal replies preserve repeated targets, full handles and justification operands', () => {
  const value = fixture(); assert.equal(validateWargoalsReply(value), value);
  assert.equal(fixed5('1750000'), '17.5'); assert.equal(fixed5('-1'), '-0.00001');
  assert.equal(fixed5('9223372036854775807'), '92233720368547.75807');
  assert.deepEqual(validateWargoalsReply(value).goals[0].states, [1, 2, 1]);
});
test('war-goal replies reject incomplete or rounded metadata instead of inventing absence', () => {
  for (const change of [
    (v: ReturnType<typeof fixture>) => { v.goals[0].states = [-1]; },
    (v: ReturnType<typeof fixture>) => { v.goals[0].handle = '18446744073709551616'; },
    (v: ReturnType<typeof fixture>) => { (v.justifications[0] as unknown as Record<string, unknown>).points = 1750000; },
    (v: ReturnType<typeof fixture>) => { delete (v.goals[0] as unknown as Record<string, unknown>).actor; },
  ]) { const value = fixture(); change(value); assert.throws(() => validateWargoalsReply(value)); }
});
test('authored term factors retain exact fixed-point strings and explicit conditions', () => {
  const value = { ...fixture(), goals: [{ ...fixture().goals[0], terms: {
    allowed: 'Country Mod republic', available: null as string | null, annex: true, baseGenerationCost: 12, perStateGenerationCost: 4, expiryDays: 90, threat: '1',
    peace: [{ name: 'Custom term', limit: 3, costFactor: '87500', threatFactor: '150000', conditioned: true }],
  } }] };
  assert.equal(validateWargoalsReply(value).goals[0].terms?.peace[0].costFactor, '87500');
  const withJustificationTerms = { ...value, justifications: [{ ...value.justifications[0], terms: value.goals[0].terms }] };
  assert.equal(validateWargoalsReply(withJustificationTerms).justifications[0].terms?.expiryDays, 90);
  const incompleteJustification = fixture();
  delete (incompleteJustification.justifications[0] as unknown as Record<string, unknown>).terms;
  assert.throws(() => validateWargoalsReply(incompleteJustification));
  (value.goals[0].terms.peace[0] as unknown as Record<string, unknown>).costFactor = 0.875;
  assert.throws(() => validateWargoalsReply(value));
  assert.throws(() => validateWargoalsReply(withJustificationTerms));
});


test('inventory action results preserve recorded blockers and unresolved actions independently', () => {
  const value = fixture();
  const reply = validateWargoalsReply(value);
  assert.equal(reply.eligibility[0].justify, false);
  assert.equal(reply.eligibility[0].declare, null);
  for (const change of [
    (v: ReturnType<typeof fixture>) => { delete (v as unknown as Record<string, unknown>).eligibility; },
    (v: ReturnType<typeof fixture>) => { (v.eligibility[0] as unknown as Record<string, unknown>).justify = 0; },
    (v: ReturnType<typeof fixture>) => { delete (v.eligibility[0] as unknown as Record<string, unknown>).declareReason; },
  ]) { const value = fixture(); change(value); assert.throws(() => validateWargoalsReply(value)); }
});

test('authored gate descriptions preserve known, empty and unavailable text', () => {
  const value = { ...fixture(), goals: [{ ...fixture().goals[0], terms: {
    allowed: 'Country Mod republic', available: null as string | null, annex: false,
    baseGenerationCost: 0, perStateGenerationCost: 0, expiryDays: 0, threat: '0', peace: [],
  } }] };
  assert.equal(validateWargoalsReply(value).goals[0].terms?.allowed, 'Country Mod republic');
  assert.equal(validateWargoalsReply(value).goals[0].terms?.available, null);
  value.goals[0].terms.available = '';
  assert.equal(validateWargoalsReply(value).goals[0].terms?.available, '');
  delete (value.goals[0].terms as unknown as Record<string, unknown>).allowed;
  assert.throws(() => validateWargoalsReply(value));
});

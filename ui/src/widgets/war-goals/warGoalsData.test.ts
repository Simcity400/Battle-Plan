import assert from 'node:assert/strict';
import test from 'node:test';
import { fixed5, validateWargoalsReply } from './warGoalsData.ts';
const fixture = () => ({ branchId: '1', hour: 80, tag: 'MOD', standings: [{ actor: 'MOD', recipient: 'TWO', standing: 'sameFaction' as string | null }, { actor: 'MOD', recipient: 'THREE', standing: null as string | null }], goals: [{ handle: '18446744073709551615', name: 'Custom goal', actor: 'MOD', recipient: 'TWO', available: true, expires: 100, states: [1, 2, 1], liberate: ['THREE', 'TWO'], puppets: ['FOUR'] }],
  justifications: [{ ordinal: 4, name: 'Custom goal', actor: 'MOD', recipient: 'TWO', states: [3, 1], total: '9000000', points: '1750000', daily: '100000', payUpfront: true, start: 50, completion: 120, paused: false }], stateNames: [{ id: 1, name: 'Custom state', known: true }], warnings: ['War-goal name refused (1)'] });
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

test('pair standings keep a recorded war or faction, and nothing when neither is proven', () => {
  const reply = validateWargoalsReply(fixture());
  assert.deepEqual(reply.standings.map(pair => pair.standing), ['sameFaction', null]);
  for (const change of [
    (v: ReturnType<typeof fixture>) => { delete (v as unknown as Record<string, unknown>).standings; },
    (v: ReturnType<typeof fixture>) => { v.standings[0].standing = 'eligible'; },
    (v: ReturnType<typeof fixture>) => { delete (v.standings[1] as unknown as Record<string, unknown>).standing; },
  ]) { const value = fixture(); change(value); assert.throws(() => validateWargoalsReply(value)); }
});

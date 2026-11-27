import assert from 'node:assert/strict';
import test from 'node:test';
import { validateWargoalsReply } from './warGoalsData.ts';
const fixture = () => ({ branchId: '1', tag: 'MOD', standings: [{ actor: 'MOD', recipient: 'TWO', standing: 'sameFaction' as string | null }, { actor: 'MOD', recipient: 'THREE', standing: null as string | null }], goals: [{ handle: '18446744073709551615', name: 'Custom goal', nameMissing: null, actor: 'MOD', recipient: 'TWO', available: true, expires: 100, states: [1, 2, 1], liberate: ['THREE', 'TWO'], puppets: ['FOUR'], state: 'ready', granted: 90 as number | null, grantedMissing: null as string | null }],
  removed: [{ handle: '5', name: 'Conquer', recipient: 'TWO', seen: 94, granted: null, grantedMissing: 'Held when the branch began', removed: 95, removedExact: true, reasonMissing: 'War goal removal reason was not recorded' }], removedUnread: null,
  justifications: [{ id: '4294967300', ordinal: 4, name: 'Custom goal', nameMissing: null, actor: 'MOD', recipient: 'TWO', states: [3, 1], total: '9000000', daily: '100000', payUpfront: true, start: 50, completion: 120, paused: false }], stateNames: [{ id: 1, name: 'Custom state', known: true }], warnings: ['War-goal name refused (1)'] });
test('war-goal replies preserve repeated targets, full handles and justification operands', () => {
  const value = fixture(); assert.equal(validateWargoalsReply(value), value);
  assert.deepEqual(validateWargoalsReply(value).goals[0].states, [1, 2, 1]);
});
test('unread war-goal names are missing with their own reason', () => {
  const value = fixture();
  const reason = 'War-goal type spelling not recorded';
  for (const row of [...value.goals, ...value.justifications]) Object.assign(row, { name: null, nameMissing: reason });
  const reply = validateWargoalsReply(value);
  assert.deepEqual([...reply.goals, ...reply.justifications].map(row => [row.name, row.nameMissing]), [[null, reason], [null, reason]]);
});
test('war-goal replies reject incomplete or rounded metadata instead of inventing absence', () => {
  for (const change of [
    (v: ReturnType<typeof fixture>) => { v.goals[0].states = [-1]; },
    (v: ReturnType<typeof fixture>) => { v.goals[0].handle = '18446744073709551616'; },
    (v: ReturnType<typeof fixture>) => { (v.justifications[0] as unknown as Record<string, unknown>).id = 4294967300; },
    (v: ReturnType<typeof fixture>) => { delete (v.goals[0] as unknown as Record<string, unknown>).actor; },
    (v: ReturnType<typeof fixture>) => { v.goals[0].state = 'unused'; },
    (v: ReturnType<typeof fixture>) => { delete (v.removed[0] as unknown as Record<string, unknown>).reasonMissing; },
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

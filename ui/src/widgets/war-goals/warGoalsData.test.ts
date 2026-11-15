import assert from 'node:assert/strict';
import test from 'node:test';
import { expiryText, fixed5, progressText, validateWargoalsReply, wargoalStructure } from './warGoalsData.ts';
import { dayLabel } from '../../charts/draw.ts';
const fixture = () => ({ branchId: '1', tag: 'MOD', standings: [{ actor: 'MOD', recipient: 'TWO', standing: 'sameFaction' as string | null }, { actor: 'MOD', recipient: 'THREE', standing: null as string | null }], goals: [{ handle: '18446744073709551615', name: 'Custom goal', actor: 'MOD', recipient: 'TWO', available: true, expires: 100, states: [1, 2, 1], liberate: ['THREE', 'TWO'], puppets: ['FOUR'] }],
  justifications: [{ id: '4294967300', ordinal: 4, name: 'Custom goal', actor: 'MOD', recipient: 'TWO', states: [3, 1], total: '9000000', daily: '100000', payUpfront: true, start: 50, completion: 120, paused: false }], stateNames: [{ id: 1, name: 'Custom state', known: true }], warnings: ['War-goal name refused (1)'] });
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
    (v: ReturnType<typeof fixture>) => { (v.justifications[0] as unknown as Record<string, unknown>).id = 4294967300; },
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

test('progress and the days to expiry are painted, never rendered for: the structure holds while they move', () => {
  const reply = validateWargoalsReply(fixture());
  const later = validateWargoalsReply({ ...fixture(), justifications: fixture().justifications.map(entry => ({ ...entry, completion: 144 })) });
  assert.equal(wargoalStructure(later), wargoalStructure(reply), 'a completion date moving with the hour renders nothing');
  const paused = validateWargoalsReply({ ...fixture(), justifications: fixture().justifications.map(entry => ({ ...entry, paused: true })) });
  assert.notEqual(wargoalStructure(paused), wargoalStructure(reply), 'a payment pause renders');
  assert.deepEqual(progressText(1_750_000, '9000000', 'Not recorded'), { share: 19.4, shareText: '19.4%', text: ' · 17.5 / 90' });
  assert.deepEqual(progressText(12_000_000, '9000000', 'Not recorded'), { share: 100, shareText: '100%', text: ' · 120 / 90' }, 'the bar is clamped, the points exact');
  assert.deepEqual(progressText(undefined, '9000000', 'Not recorded at this hour'), { share: null, shareText: '', text: 'Not recorded at this hour / 90' }, 'never zero');
  assert.equal(expiryText(100, 80), `Expires ${dayLabel(100)} · 1 day`);
  assert.equal(expiryText(100, 100), `Expiry date passed ${dayLabel(100)}`);
});

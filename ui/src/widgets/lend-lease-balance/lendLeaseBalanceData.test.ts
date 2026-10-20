import test from 'node:test';
import assert from 'node:assert/strict';
import type { LendLeaseBalanceReply } from '../../generated/dashboard';
import { amountText, emptyBalance, rankedAt, validateLendLeaseBalance } from './lendLeaseBalanceData.ts';

const history = (holder: string, tag: string, sent: string, received: string, from = 0) => ({
  holder, tags: [{ hour: from, tag }], values: [[[from, 50, sent]], [], [[from, 50, received]], []],
});
const reply = (): LendLeaseBalanceReply => ({
  branchId: '1', lastHour: 50, scale: 100000, holder: null, warnings: [], known: [[0, 50, '1']],
  histories: [history('1', 'AAA', '500000', '0'), history('2', 'BBB', '0', '300000'), history('3', 'CCC', '200000', '100000'),
    history('4', 'DDD', '0', '0'), history('5', 'EEE', '-100', '0', 20)],
  coverage: { firstHour: 0, lastHour: 50, complete: true }, pending: [],
} as unknown as LendLeaseBalanceReply);

test('ranking uses sent plus received, ties by holder, and countries with neither leave', () => {
  const data = validateLendLeaseBalance(reply());
  const all = rankedAt(data, 'industry', 10, { kind: 'all' }, [], undefined, 50)!;
  assert.deepEqual(all.map(row => row.tag), ['AAA', 'BBB', 'CCC']);
  const top = rankedAt(data, 'industry', 10, { kind: 'group', group: { type: 'top', count: 2 }, membership: 'hour', aggregate: false }, [], undefined, 50)!;
  assert.deepEqual(top.map(row => row.tag), ['AAA', 'BBB']);
  // Before its first read a history is unknown, not zero; a signed correction stays.
  assert.ok(!rankedAt(data, 'industry', 10, { kind: 'all' }, [], undefined, 50)!.some(row => row.tag === 'EEE'));
  const later = rankedAt(data, 'industry', 30, { kind: 'tags', tags: ['EEE'] }, [], undefined, 50)!;
  assert.equal(amountText(later[0].sent, 100000), '-0.001');
  // Fuel was not recorded for anyone here.
  assert.deepEqual(rankedAt(data, 'fuel', 10, { kind: 'all' }, [], undefined, 50), []);
});

test('amounts stay exact', () => {
  assert.equal(amountText(123456789n, 100000), '1,234.56789');
  assert.equal(amountText(null, 100000), '—');
});

test('no lend-lease is said only for recorded zero; unread or unrostered is unavailable', () => {
  const data = validateLendLeaseBalance(reply());
  assert.equal(emptyBalance(data, 'industry', 10, { kind: 'tags', tags: ['DDD'] }, [], undefined, 50), 'zero');
  assert.equal(emptyBalance(data, 'industry', 10, { kind: 'tags', tags: ['ZZZ'] }, [], undefined, 50), 'none-recorded');
  assert.equal(emptyBalance({ ...data, known: [] }, 'industry', 10, { kind: 'all' }, [], undefined, 50), 'unrecorded');
  // One recorded zero never hides a selected country with no recorded total.
  assert.equal(emptyBalance(data, 'industry', 10, { kind: 'tags', tags: ['DDD', 'ZZZ'] }, [], undefined, 50), 'partial');
  // A fuel total not recorded is not zero, though the industry one is.
  assert.equal(emptyBalance(data, 'fuel', 10, { kind: 'tags', tags: ['DDD'] }, [], undefined, 50), 'none-recorded');
});

test('a group proves no lend-lease only from its own members\' histories', () => {
  const data = validateLendLeaseBalance(reply());
  const groups = (members: string[], unobserved: { firstHour: number; lastHour: number }[] = []) => ({ humans: [], wars: [],
    factions: [{ id: 'g', members: members.map(tag => ({ tag, firstHour: 0, lastHour: 50 })), incomplete: [] }],
    unobserved: { humans: [], factions: unobserved, wars: [] } }) as unknown as Parameters<typeof emptyBalance>[5];
  const group = { kind: 'group', group: { type: 'faction', id: 'g' }, membership: 'hour', aggregate: false } as unknown as Parameters<typeof emptyBalance>[3];
  // DDD's recorded zero says nothing about a group of ZZZ.
  assert.equal(emptyBalance(data, 'industry', 10, group, [], groups(['ZZZ']), 50), 'none-recorded');
  assert.equal(emptyBalance(data, 'industry', 10, group, [], groups(['DDD']), 50), 'zero');
  assert.equal(emptyBalance(data, 'industry', 10, group, [], groups(['DDD', 'ZZZ']), 50), 'partial');
  // Groups still loading prove nothing; nor does a roster not observed at the hour.
  assert.equal(emptyBalance(data, 'industry', 10, group, [], undefined, 50), 'none-recorded');
  assert.equal(emptyBalance(data, 'industry', 10, group, [], groups(['DDD'], [{ firstHour: 5, lastHour: 15 }]), 50), 'members-unrecorded');
});

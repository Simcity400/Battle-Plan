import test from 'node:test';
import assert from 'node:assert/strict';
import { amountLabel, balanceRows, balanceSelection, emptyBalance, firstRecorded, partnersAt, rankedRows, totalsOverTime, validateLendLeaseBalance,
  validateLendLeasePartners, valueAt } from './lendLeaseBalanceData.ts';
import type { LendLeaseBalanceReply, LendLeasePartnersReply } from '../../generated/dashboard';

const partners = (): LendLeasePartnersReply => ({
  holder: 'GER', scale: 100000, firstHour: 0, lastHour: 99, warnings: [], known: [{ firstHour: 0, lastHour: 39 }, { firstHour: 50, lastHour: 99 }],
  totals: [[[10, 29, '300000'], [30, 39, '100000'], [50, 99, '100000']], [[10, 99, '0']]],
  partners: [
    { slot: 8, tags: [{ firstHour: 10, lastHour: 99, tag: 'FRA' }], values: [[[10, 29, '100000'], [30, 39, '100000'], [50, 99, '100000']], [[10, 99, '0']]] },
    { slot: 4, tags: [{ firstHour: 20, lastHour: 99, tag: 'ITA' }], values: [[[20, 29, '200000'], [30, 39, '0'], [50, 99, '0']], [[20, 99, '0']]] },
  ],
});

test('a holder\'s partners at an hour are read from their runs: ranked, held at zero once their row leaves, unknown outside the reads', () => {
  const data = validateLendLeasePartners(partners());
  assert.deepEqual(partnersAt(data, 25).map(row => [row.tag, row.sent]), [['ITA', 200000n], ['FRA', 100000n]]);
  assert.deepEqual(partnersAt(data, 35).map(row => [row.tag, row.sent]), [['FRA', 100000n], ['ITA', 0n]], 'a partner gone from the histories stays, at zero');
  assert.deepEqual(partnersAt(data, 45), [], 'an unread hour shows no partner, never zeros');
  assert.deepEqual(partnersAt(data, 15).map(row => row.tag), ['FRA'], 'a partner shows from its first row on');
  assert.equal(valueAt(data.totals[0], 40), null);
  assert.equal(firstRecorded(data), 10);
  assert.deepEqual(totalsOverTime(data).sent.map(run => [run.firstHour, run.value]), [[10, 3], [30, 1], [50, 1]]);
  assert.throws(() => validateLendLeasePartners({ ...partners(), totals: [[[5, 1, '1']], []] }), /malformed/);
});

const reply = (): LendLeaseBalanceReply => ({
  holder: null, scale: 100000, missing: null, warnings: [],
  rows: [
    { slot: 2, tag: 'ENG', values: ['500000', '0', '0', '7'] },
    { slot: 3, tag: 'SOV', values: ['0', '0', '900000', '0'] },
    { slot: 5, tag: 'FRA', values: ['0', '0', '0', '0'] },
    { slot: 6, tag: null, values: ['9', '9', '9', '9'] },
  ],
});

test('rows rank by sent plus received, drop countries with neither and those without a tag, and keep the selection', () => {
  const rows = balanceRows(validateLendLeaseBalance(reply()), 'industry');
  assert.deepEqual(rankedRows(rows, { tags: null, limit: Infinity }).map(row => row.tag), ['SOV', 'ENG']);
  assert.deepEqual(rankedRows(rows, { tags: null, limit: 1 }).map(row => row.tag), ['SOV']);
  assert.deepEqual(rankedRows(rows, { tags: new Set(['ENG']), limit: Infinity }).map(row => row.tag), ['ENG']);
  assert.deepEqual(balanceRows(reply(), 'fuel').map(row => [row.tag, row.sent, row.received]), [['ENG', 0n, 7n], ['SOV', 0n, 0n], ['FRA', 0n, 0n]]);
});

test('an empty chart says whether zero is proven by the rosters or nothing is recorded', () => {
  const rows = balanceRows(reply(), 'industry');
  assert.equal(emptyBalance(rows, { tags: new Set(['FRA']) }), 'zero');
  assert.equal(emptyBalance(rows, { tags: new Set(['FRA', 'GER']) }), 'partial');
  assert.equal(emptyBalance(rows, { tags: new Set(['GER']) }), 'none-recorded');
  assert.equal(emptyBalance(rows, { tags: new Set(), membersUnknown: true }), 'members-unrecorded');
  assert.deepEqual(balanceSelection({ kind: 'tags', tags: ['ENG'] }, [], undefined, 0, 0), { tags: new Set(['ENG']), limit: Infinity });
});

test('labels keep two decimals unless that would read zero, and a malformed reply is refused', () => {
  assert.equal(amountLabel(123456n), '1.23');
  assert.equal(amountLabel(7n), '0.00007');
  assert.throws(() => validateLendLeaseBalance({ ...reply(), rows: [...reply().rows].reverse() }));
});

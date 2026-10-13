import test from 'node:test';
import assert from 'node:assert/strict';
import type { LedgerDaily, LedgerSpend, PoliticsLedgerReply } from '../../generated/dashboard';
import { runAt } from '../../charts/series.ts';
import { powerBalanceIndex } from '../../dashboard/powerBalance.ts';
import { powerActionDetails, powerCountrySegments, powerPartHistory, powerPartsAt } from './powerDrillData.ts';

const daily: LedgerDaily = { country: 1, instance: 1, epoch: '1', dayHour: 24, readHour: 26,
  ppGrossRaw: '200001', ppCostRaw: '-50001', ppIncomeRaw: '250002', ppRealizedRaw: '0', ppSequence: '1',
  cpIncomeRaw: '-120001', cpRealizedRaw: '0', cpSequence: '2' };
const spend: LedgerSpend = { id: 'payment', sequence: '3', ordinal: 0, hour: 24, readHour: 26, countryTag: 'mod country', countryName: null,
  currency: 'Political', category: 'advisors', itemId: 'advisor', chargeRaw: '9', realizedDeltaRaw: '0', covered: true, occurrences: 2,
  changes: [{ hour: 24, readHour: 26, chargeRaw: '4', realizedDeltaRaw: '0', covered: true, occurrences: 1 },
    { hour: 27, readHour: 28, chargeRaw: '5', realizedDeltaRaw: '0', covered: true, occurrences: 1 }] };
const reply = (overrides: Partial<PoliticsLedgerReply> = {}): PoliticsLedgerReply => ({ branchId: '1', tag: 'mod country', decimals: 5,
  daily: [daily], warnings: [], spending: [spend], spendItems: [{ id: 'advisor', groupId: 'advisor identity', name: 'Recorded advisor',
    action: 'Hired a recorded advisor', icon: null, tooltipToken: null, character: 7, targetTag: null, targetName: null, unknown: null }],
  spendCategories: [{ id: 'advisors', name: 'Advisors', identified: true }],
  spendingCoverage: [{ readHour: 24, sinceHour: 24, throughHour: 30, political: true, command: true, reason: null }],
  ideaTooltips: null, commandHolds: [], ...overrides });

test('income drill retains signed native operands and wrapping, separate from the realized application', () => {
  const index = powerBalanceIndex(reply(), 'Political');
  assert.equal(powerPartsAt(index, 'income', 25, 30).total, null);
  const recorded = powerPartsAt(index, 'income', 26, 30);
  assert.deepEqual(recorded.parts.map(part => part.raw), [200001n, 50001n, 0n]);
  assert.equal(recorded.total, 250002n);
  assert.equal(powerPartsAt(powerBalanceIndex(reply(), 'Command'), 'income', 26, 30).total, -120001n);
  const wrapped = powerBalanceIndex(reply({ daily: [{ ...daily, ppGrossRaw: '9223372036854775807', ppCostRaw: '-1',
    ppIncomeRaw: '-9223372036854775808' }] }), 'Political');
  assert.equal(powerPartsAt(wrapped, 'income', 26, 30).parts[2].raw, -(1n << 64n));
  assert.equal(powerPartsAt(wrapped, 'income', 26, 30).total, -(1n << 63n));
});

test('daily spending drill never reveals future contributions or fills an interval gap', () => {
  const index = powerBalanceIndex(reply(), 'Political');
  assert.equal(powerPartsAt(index, 'spending', 26, 30).total, 4n);
  assert.equal(powerPartsAt(index, 'spending', 28, 30).total, 9n);
  assert.equal(powerPartsAt(index, 'spending', 30, 26).total, 4n);
  const gap = powerBalanceIndex(reply({ spendingCoverage: [{ readHour: 26, sinceHour: 26, throughHour: 30,
    political: true, command: true, reason: null }] }), 'Political');
  assert.equal(powerPartsAt(gap, 'spending', 28, 30).parts[0].raw, 9n);
  assert.equal(powerPartsAt(gap, 'spending', 28, 30).total, null);
  const deficit = powerBalanceIndex(reply({ spending: [{ ...spend, currency: 'Command', category: 'dailyDeficit' }],
    spendCategories: [{ id: 'dailyDeficit', name: 'Daily deficit', identified: true }] }), 'Command');
  assert.equal(powerPartsAt(deficit, 'spending', 28, 30).total, 0n);
  assert.equal(powerPartsAt(deficit, 'spending', 28, 30).parts.length, 0);
});

test('part history starts at the supplying read, ends coverage honestly and obeys membership', () => {
  const index = powerBalanceIndex(reply(), 'Political');
  const income = powerPartHistory(index, 'income', null);
  assert.equal(runAt(income[0].runs, 25), null);
  assert.equal(runAt(income[0].runs, 26)?.value, 2.00001);
  assert.equal(runAt(income[0].runs, 48), null);
  const spending = powerPartHistory(index, 'spending', [{ firstHour: 27, lastHour: 29 }]);
  assert.equal(runAt(spending[0].runs, 26), null);
  assert.equal(runAt(spending[0].runs, 27)?.value, 0.00004);
  assert.equal(runAt(spending[0].runs, 28)?.value, 0.00009);
  assert.equal(runAt(spending[0].runs, 31), null);
});

test('an identical balance across a reused tag still starts a separate income stretch', () => {
  const countries = powerCountrySegments([
    { firstHour: 24, lastHour: 26, valueRaw: '10', startsSegment: true },
    { firstHour: 27, lastHour: 30, valueRaw: '10', startsSegment: true },
  ]);
  const index = powerBalanceIndex(reply(), 'Political');
  assert.equal(powerPartsAt(index, 'income', 26, 30, countries).total, 250002n);
  assert.equal(powerPartsAt(index, 'income', 27, 30, countries).total, null);
  assert.equal(powerPartsAt(index, 'spending', 28, 30, countries).total, null);
  assert.equal(runAt(powerPartHistory(index, 'income', null, countries)[0].runs, 27), null);
});

test('income bars and history retain each currency supplying read in either arrival order', () => {
  for (const ppFirst of [true, false]) {
    const data = reply({ daily: [{ ...daily, readHour: 25, ppReadHour: ppFirst ? 24 : 25,
      cpReadHour: ppFirst ? 25 : 24 }], spending: [] });
    const early = powerBalanceIndex(data, ppFirst ? 'Political' : 'Command');
    const later = powerBalanceIndex(data, ppFirst ? 'Command' : 'Political');
    assert.equal(powerPartsAt(early, 'income', 24, 30).total, ppFirst ? 250002n : -120001n);
    assert.equal(powerPartsAt(later, 'income', 24, 30).total, null);
    assert.equal(powerPartsAt(later, 'income', 25, 30).total, ppFirst ? -120001n : 250002n);
    assert.notEqual(runAt(powerPartHistory(early, 'income', null)[0].runs, 24), null);
    assert.equal(runAt(powerPartHistory(later, 'income', null)[0].runs, 24), null);
    assert.notEqual(runAt(powerPartHistory(later, 'income', null)[0].runs, 25), null);
  }
});

test('an entirely absent command tuple creates neither dated bars nor history', () => {
  const index = powerBalanceIndex(reply({ daily: [{ ...daily, cpIncomeRaw: null,
    cpRealizedRaw: null, cpSequence: null, cpReadHour: null }] }), 'Command');
  assert.deepEqual(powerPartsAt(index, 'income', 26, 30).parts, []);
  assert.equal(powerPartsAt(index, 'income', 26, 30).total, null);
  assert.deepEqual(powerPartHistory(index, 'income', null)[0].runs, []);
});

test('missing income and spending use concise statuses without backend reasons', () => {
  const none = powerBalanceIndex(reply({ daily: [] }), 'Political');
  assert.equal(powerPartsAt(none, 'income', 26, 30).reason, 'Income not recorded.');
  const incomplete = powerBalanceIndex(reply({ daily: [{ ...daily, ppGrossRaw: null }] }), 'Political');
  assert.equal(powerPartsAt(incomplete, 'income', 26, 30).total, 250002n);
  assert.equal(powerPartsAt(incomplete, 'income', 26, 30).reason, 'Income breakdown incomplete.');
  const unknown = powerBalanceIndex(reply({ spendingCoverage: [{ readHour: 24, sinceHour: null,
    throughHour: 30, political: false, command: false, reason: 'CountryPolitics native operand unavailable' }] }), 'Political');
  assert.equal(powerPartsAt(unknown, 'spending', 26, 30).reason, 'Spending total is incomplete.');
  assert.equal(powerPartsAt(incomplete, 'income', 26, 30).parts[2].label, 'Adjustment');
});

test('an incomplete payment qualifies its known total without discarding its exact amount', () => {
  const data = reply({ spending: [{ ...spend, changes: spend.changes.map(change => ({ ...change, covered: false })) }] });
  const parts = powerPartsAt(powerBalanceIndex(data, 'Political'), 'spending', 26, 30);
  assert.equal(parts.parts[0].raw, 4n);
  assert.equal(parts.total, 4n);
  assert.equal(parts.reason, 'Spending total is incomplete.');
});

test('drill details name only what the parts do not show, never a future publication or a repeated change', () => {
  const index = powerBalanceIndex(reply({ daily: [], balanceChanges: [{ hour: 24, readHour: 26, currency: 'Political', action: 'Synthetic adjustment',
    amountRaw: '9', realizedDeltaRaw: '4', covered: true, reason: null }] }), 'Political');
  assert.deepEqual(powerActionDetails(index, 26, 30), [], 'recorded changes are not repeated beside the parts');
  const capped = powerBalanceIndex(reply({ daily: [{ country: 1, instance: 1, epoch: '1', dayHour: 24, readHour: 26, ppGrossRaw: '300000', ppCostRaw: '0',
    ppIncomeRaw: '300000', ppRealizedRaw: '100000', ppSequence: '1', cpIncomeRaw: null, cpRealizedRaw: null, cpSequence: null }] }), 'Political');
  assert.deepEqual(powerActionDetails(capped, 25, 30), []);
  assert.deepEqual(powerActionDetails(capped, 26, 30), [{ label: 'Lost at the cap', value: '−2.00' }]);
  assert.deepEqual(powerActionDetails(capped, 26, 25), []);
  const holds = [{ readHour: 0, throughHour: 25, totalRaw: '-500000', holders: [], reason: null },
    { readHour: 26, throughHour: 30, totalRaw: '-300000', holders: [], reason: null }];
  const command = powerBalanceIndex(reply({ commandHolds: holds }), 'Command');
  assert.deepEqual(powerActionDetails(command, 26, 30), [{ label: 'Reserved', value: '−2' }]);
  assert.equal(powerActionDetails(command, 26, 30).some(row => row.label === 'Total held'), false);
});

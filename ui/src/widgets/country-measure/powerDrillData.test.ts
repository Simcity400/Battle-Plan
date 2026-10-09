import test from 'node:test';
import assert from 'node:assert/strict';
import type { LedgerDaily, LedgerSpend, PoliticsLedgerReply } from '../../generated/dashboard';
import { runAt } from '../../charts/series.ts';
import { powerBalanceIndex } from '../../dashboard/powerBalance.ts';
import { powerCountrySegments, powerPartHistory, powerPartsAt } from './powerDrillData.ts';

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

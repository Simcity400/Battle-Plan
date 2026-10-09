import assert from 'node:assert/strict';
import test from 'node:test';
import { powerBalanceIndex } from '../../dashboard/powerBalance.ts';
import type { PoliticsLedgerReply } from '../../generated/dashboard';
import { powerCountryLedger } from './powerCountryData.ts';
import { powerPartsAt } from './powerDrillData.ts';

test('power aliases retain a native daily operand across a rename within its day and exclude a replacement', () => {
  const reply = (tag: string): PoliticsLedgerReply => ({ tag, branchId: 'b', decimals: 5, warnings: [], spending: [], spendItems: [],
    spendCategories: [], spendingCoverage: [], commandHolds: [], ideaTooltips: null });
  const first = reply('X'), renamed = reply('Y');
  const day = (country: number, amount: string) => ({ country, instance: country, epoch: '1', dayHour: 96, readHour: 97,
    ppGrossRaw: amount, ppCostRaw: '1', ppIncomeRaw: (BigInt(amount) - 1n).toString(), ppRealizedRaw: amount, ppSequence: '1',
    cpIncomeRaw: amount, cpRealizedRaw: amount, cpSequence: '1' });
  first.daily = [day(10, '300001'), day(20, '999999')];
  first.spendingCoverage = [{ sinceHour: 96, throughHour: 99, readHour: 96, political: true, command: true, reason: null }];
  renamed.spendingCoverage = [{ sinceHour: 100, throughHour: 110, readHour: 100, political: true, command: true, reason: null }];
  const spend = (countryId: number, hour: number) => ({ countryId, id: 'payment', itemId: 'item', sequence: '1', ordinal: 0,
    hour, readHour: hour + 1, countryTag: 'X', countryName: null, currency: 'Political', category: 'costs', chargeRaw: '7',
    realizedDeltaRaw: '-7', covered: true, occurrences: 1, changes: [{ hour, readHour: hour + 1, chargeRaw: '7',
      realizedDeltaRaw: '-7', covered: true, occurrences: 1 }] });
  const change = (countryId: number, hour: number) => ({ countryId, hour, readHour: hour + 1, currency: 'Political',
    amountRaw: '9', realizedDeltaRaw: '9', action: 'Recorded adjustment', covered: true, reason: null });
  for (const [ledger, hour] of [[first, 98], [renamed, 100]] as const) {
    ledger.spending = [spend(10, hour), spend(20, hour)];
    ledger.balanceChanges = [change(10, hour), change(20, hour)];
  }
  const spans = new Map([['X', [{ firstHour: 96, lastHour: 99 }]], ['Y', [{ firstHour: 100, lastHour: 110 }]]]);
  const others = Array.from({ length: 5000 }, (_, index) => ({ ...reply(`other-${index}`), warnings: ['Unrelated country warning'],
    spendItems: [{ id: `other-item-${index}` }] as PoliticsLedgerReply['spendItems'] }));
  const merged = powerCountryLedger([...others, first, renamed], spans, 'X', 10)!;
  assert.deepEqual(merged.spendItems, [], 'the chosen drill does not copy other countries\' items');
  assert.deepEqual(merged.warnings, [], 'unrelated countries do not supply the drill\'s warnings');
  assert.deepEqual(merged.daily?.map(row => row.country), [10]);
  assert.deepEqual(merged.spending.map(row => [row.countryId, row.hour, row.readHour]), [[10, 98, 99], [10, 100, 101]]);
  assert.equal(new Set(merged.spending.map(row => row.id)).size, 2, 'alias-local IDs cannot collide');
  assert.deepEqual(merged.balanceChanges?.map(row => [row.countryId, row.hour, row.readHour]), [[10, 98, 99], [10, 100, 101]]);
  assert.deepEqual(merged.spendingCoverage.map(row => [row.sinceHour, row.throughHour]), [[96, 110]]);
  const coverage = [{ firstHour: 96, lastHour: 110 }];
  assert.equal(powerPartsAt(powerBalanceIndex(merged, 'Political'), 'income', 102, 102, coverage).total, 300000n);
  assert.equal(powerPartsAt(powerBalanceIndex(merged, 'Command'), 'income', 102, 102, coverage).total, 300001n);
  assert.equal(powerPartsAt(powerBalanceIndex(merged, 'Command'), 'income', 111, 111, coverage).total, null);
});

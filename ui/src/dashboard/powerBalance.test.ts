import test from 'node:test';
import assert from 'node:assert/strict';
import type { LedgerBalanceChange, LedgerDaily, LedgerSpend, PoliticsLedgerReply } from '../generated/dashboard';
import { powerBalanceIndex, powerBalanceKeyHours, powerBalanceRows, powerCoverageAt, powerSpendTicks, validatePowerBalance } from './powerBalance.ts';

const change = (overrides: Partial<LedgerBalanceChange> = {}): LedgerBalanceChange => ({ hour: 24, readHour: 26, currency: 'Political',
  amountRaw: '-15000001', realizedDeltaRaw: '-10000000', action: 'Hired a recorded advisor', covered: true, reason: null, ...overrides });
const daily = (overrides: Partial<LedgerDaily> = {}): LedgerDaily => ({ country: 1, instance: 1, epoch: '1', dayHour: 24, readHour: 26,
  ppGrossRaw: '200001', ppCostRaw: '-50001', ppIncomeRaw: '250002', ppRealizedRaw: '250002', ppSequence: '1',
  cpIncomeRaw: '-120001', cpRealizedRaw: '-120001', cpSequence: '2', ...overrides });
const spend = (overrides: Partial<LedgerSpend> = {}): LedgerSpend => ({ id: 'payment', sequence: '3', ordinal: 0, hour: 24, readHour: 26,
  countryTag: 'mod country', countryName: 'Old recorded name', currency: 'Political', category: 'advisors', itemId: 'advisor',
  chargeRaw: '15000001', realizedDeltaRaw: '-10000000', covered: true, occurrences: 1,
  changes: [{ readHour: 26, chargeRaw: '15000001', realizedDeltaRaw: '-10000000', covered: true, occurrences: 1 }], ...overrides });
const reply = (overrides: Partial<PoliticsLedgerReply> = {}): PoliticsLedgerReply => ({ branchId: '1', tag: 'mod country', decimals: 5,
  daily: [daily()], balanceChanges: [change()], warnings: [], spending: [spend()],
  spendItems: [{ id: 'advisor', groupId: 'advisor identity', name: 'Recorded advisor', action: 'Hired a recorded advisor', icon: null,
    tooltipToken: null, character: 7, targetTag: null, targetName: null, unknown: null }],
  spendCategories: [{ id: 'advisors', name: 'Advisors', identified: true }],
  spendingCoverage: [{ readHour: 24, sinceHour: 24, throughHour: 30, political: true, command: true, reason: null }],
  ideaTooltips: null, commandHolds: [], ...overrides });

test('balance operands are signed and exact, with evaluated and realized amounts kept apart', () => {
  const index = powerBalanceIndex(validatePowerBalance(reply()), 'Political');
  const rows = powerBalanceRows(index, 26, 26);
  assert.equal(rows.find(row => row.label === 'Daily income')?.value, '+2.00001');
  assert.equal(rows.find(row => row.label === 'Daily costs')?.value, '−0.50001');
  assert.equal(rows.find(row => row.label === 'Net daily income')?.value, '+2.50002');
  assert.equal(rows.find(row => row.label === 'Adjustment')?.value, '−150.00001');
  assert.equal(rows.find(row => row.label.startsWith('Hired a recorded advisor ·'))?.value, '−100');
  assert.equal(rows.find(row => row.label === 'Hired a recorded advisor')?.value, '150.00001');
});

test('a later observation never supplies income or a cause at an earlier hour', () => {
  const index = powerBalanceIndex(reply(), 'Political');
  for (const rows of [powerBalanceRows(index, 25, 30), powerBalanceRows(index, 30, 25)]) {
    assert.equal(rows.find(row => row.label === 'Daily income')?.value, 'Not recorded');
    assert.equal(rows.some(row => row.label.includes('recorded advisor')), false);
  }
  assert.deepEqual(powerSpendTicks(index, [], 25), []);
  assert.deepEqual(powerSpendTicks(index, [], 26), [{ hour: 24 }]);
});

test('a payment gets a tick even when income hides the net drop; complete holds do not', () => {
  const pp = powerBalanceIndex(reply(), 'Political');
  assert.deepEqual(powerSpendTicks(pp, [], 30), [{ hour: 24 }]);
  const drops = [{ hour: 27, beforeRaw: '500000', afterRaw: '400000' }];
  const cp = powerBalanceIndex(reply({ spending: [] }), 'Command');
  assert.deepEqual(powerSpendTicks(cp, drops, 30), []);
  assert.deepEqual(powerSpendTicks(undefined, drops, 30), [{ hour: 27 }]);
});

test('capture gaps do not become zero or borrow a neighboring coverage witness', () => {
  const index = powerBalanceIndex(reply({ daily: [], spending: [], balanceChanges: [], spendingCoverage: [
    { readHour: 24, sinceHour: 24, throughHour: 25, political: true, command: true, reason: null },
    { readHour: 28, sinceHour: 28, throughHour: 30, political: true, command: true, reason: null },
  ] }), 'Political');
  assert.equal(powerCoverageAt(index, 26), null);
  const rows = powerBalanceRows(index, 26, 30);
  assert.equal(rows.find(row => row.label === 'Daily income')?.value, 'Not recorded');
  assert.ok(rows.some(row => row.label === 'Spending unavailable at this time.'));
  assert.deepEqual(powerSpendTicks(index, [{ hour: 26, beforeRaw: '2', afterRaw: '1' }], 30), [{ hour: 26 }]);
});

test('daily rates never cross into another game day, including negative engine hours', () => {
  const data = reply({ daily: [daily({ dayHour: -1, readHour: -1 })], spending: [], balanceChanges: [] });
  const index = powerBalanceIndex(data, 'Political');
  assert.equal(powerBalanceRows(index, -1, -1).find(row => row.label === 'Daily income')?.value, '+2.00001');
  assert.equal(powerBalanceRows(index, 0, 0).find(row => row.label === 'Daily income')?.value, 'Not recorded');
});

test('command deficit and current reservations remain separate from charged spending', () => {
  const index = powerBalanceIndex(reply({ spending: [spend({ currency: 'Command', category: 'dailyDeficit', chargeRaw: '120001',
    changes: [{ readHour: 26, chargeRaw: '120001', realizedDeltaRaw: '-120001', covered: true, occurrences: 1 }] })],
    spendCategories: [{ id: 'dailyDeficit', name: 'Daily deficit', identified: true }],
    commandHolds: [{ readHour: 24, throughHour: 26, totalRaw: '300001', reason: null,
    holders: [{ id: 'holder', kind: 'Ability', name: 'Recorded ability', amountRaw: '300001' }] }] }), 'Command');
  const rows = powerBalanceRows(index, 26, 26);
  assert.equal(rows.find(row => row.label === 'Daily deficit')?.value, '−1.20001');
  assert.equal(rows.find(row => row.label === 'Total held')?.value, '3.00001');
  assert.equal(rows.some(row => row.label === 'Spending that day'), false);
  assert.deepEqual(powerSpendTicks(index, [], 26), []);
  assert.equal(powerBalanceRows(index, 27, 27).find(row => row.label === 'Total held')?.value, 'Not known');
});

test('malformed signed operands and future-dated application hours are refused', () => {
  for (const data of [reply({ daily: [daily({ ppGrossRaw: '1.2' })] }), reply({ daily: [daily({ readHour: 23 })] }),
    reply({ balanceChanges: [change({ readHour: 23 })] }), reply({ balanceChanges: [change({ realizedDeltaRaw: (1n << 127n).toString() })] })]) {
    assert.throws(() => validatePowerBalance(data));
  }
  assert.equal(validatePowerBalance(reply({ daily: [daily({ ppGrossRaw: '-9223372036854775808' })] })).daily![0].ppGrossRaw, '-9223372036854775808');
});

test('read-dated daily upkeep includes only increments visible at the chosen hour', () => {
  const data = reply({ spending: [spend({ currency: 'Command', changes: [
    { readHour: 26, chargeRaw: '1', realizedDeltaRaw: '-1', covered: true, occurrences: 1 },
    { readHour: 28, chargeRaw: '2', realizedDeltaRaw: '-2', covered: true, occurrences: 1 },
  ] })] });
  const index = powerBalanceIndex(data, 'Command');
  assert.equal(powerBalanceRows(index, 26, 30).find(row => row.label === 'Hired a recorded advisor')?.value, '0.00001');
  assert.equal(powerBalanceRows(index, 28, 30).find(row => row.label === 'Hired a recorded advisor')?.value, '0.00003');
  assert.deepEqual(powerBalanceKeyHours(index), [26, 28]);
});

test('one daily currency arriving later cannot hide or backdate the other currency', () => {
  for (const ppFirst of [true, false]) {
    const data = validatePowerBalance(reply({ daily: [daily({ readHour: 25,
      ppReadHour: ppFirst ? 24 : 25, cpReadHour: ppFirst ? 25 : 24 })], spending: [], balanceChanges: [] }));
    const early = powerBalanceIndex(data, ppFirst ? 'Political' : 'Command');
    const later = powerBalanceIndex(data, ppFirst ? 'Command' : 'Political');
    assert.equal(powerBalanceRows(early, 24, 30).find(row => row.label === (ppFirst ? 'Daily income' : 'Daily deficit'))?.value,
      ppFirst ? '+2.00001' : '−1.20001');
    assert.equal(powerBalanceRows(later, 24, 30).find(row => row.label === 'Daily income')?.value, 'Not recorded');
    assert.notEqual(powerBalanceRows(later, 25, 30).find(row => row.label === (ppFirst ? 'Daily deficit' : 'Daily income'))?.value,
      'Not recorded');
    assert.deepEqual(powerBalanceKeyHours(early), [24]);
    assert.deepEqual(powerBalanceKeyHours(later), [25]);
  }
});

test('daily currency read dates cannot precede the native day or exceed the row read', () => {
  for (const overrides of [{ ppReadHour: 23 }, { cpReadHour: 27 }]) {
    assert.throws(() => validatePowerBalance(reply({ daily: [daily(overrides)] })));
  }
});

test('an absent command tuple supplies no application date, while a known application may lack its operand', () => {
  const absent = daily({ cpIncomeRaw: null, cpRealizedRaw: null, cpSequence: null, cpReadHour: null });
  const index = powerBalanceIndex(reply({ daily: [absent], balanceChanges: [], spending: [] }), 'Command');
  const rows = powerBalanceRows(index, 26, 30);
  assert.equal(rows.find(row => row.label === 'Daily income')?.value, 'Not recorded');
  assert.equal(rows.some(row => row.label === 'Income applied / read'), false);
  assert.deepEqual(powerBalanceKeyHours(index), []);
  for (const application of [{ cpRealizedRaw: '0' }, { cpSequence: '2' }]) {
    const known = powerBalanceIndex(reply({ daily: [{ ...absent, ...application }], balanceChanges: [], spending: [] }), 'Command');
    assert.equal(powerBalanceRows(known, 26, 30).find(row => row.label === 'Daily income')?.value, 'Not recorded');
    assert.ok(!powerBalanceRows(known, 26, 30).some(row => row.label === 'Income applied / read'));
    assert.equal(powerBalanceRows(known, 26, 30).find(row => row.label === 'Daily income')?.value, 'Not recorded');
    assert.deepEqual(powerBalanceKeyHours(known), [26]);
  }
});


test('several missing changes have one plain status while action values remain separate', () => {
  const index = powerBalanceIndex(reply({ balanceChanges: [change({action: 'First', covered: false, reason: 'Synthetic private reason'}),
    change({action: 'Second', covered: false, reason: 'Another private reason'})] }), 'Political');
  const rows = powerBalanceRows(index, 26, 30);
  assert.equal(rows.filter(row => row.label === 'Some changes are missing.').length, 1);
  assert.equal(rows.filter(row => row.label.startsWith('First ·') || row.label.startsWith('Second ·')).length, 2);
  assert.equal(rows.some(row => /private reason|Change read|Income applied/.test(row.label)), false);
});

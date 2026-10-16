import test from 'node:test';
import assert from 'node:assert/strict';
import type { LedgerBalanceChange, LedgerCommandHold, LedgerDaily, LedgerSpend, PoliticsLedgerReply } from '../generated/dashboard';
import { powerBalanceIndex, powerBalanceKeyHours, powerBalanceRows, powerCoverageAt, powerGain, powerSpendTicks, reserveMoves, validatePowerBalance } from './powerBalance.ts';

const change = (overrides: Partial<LedgerBalanceChange> = {}): LedgerBalanceChange => ({ hour: 24, readHour: 26, currency: 'Political',
  amountRaw: '-15012345', realizedDeltaRaw: '-10000000', action: 'Hired a recorded advisor', covered: true, reason: null, ...overrides });
// Gain 2.66 before 1.17 of daily costs: a net change of 1.49 a day, as the game adds it.
const daily = (overrides: Partial<LedgerDaily> = {}): LedgerDaily => ({ country: 1, instance: 1, epoch: '1', dayHour: 24, readHour: 26,
  ppGrossRaw: '266000', ppCostRaw: '117000', ppIncomeRaw: '149000', ppRealizedRaw: '149000', ppSequence: '1',
  cpIncomeRaw: '-120001', cpRealizedRaw: '-120001', cpSequence: '2', ...overrides });
const spend = (overrides: Partial<LedgerSpend> = {}): LedgerSpend => ({ id: 'payment', sequence: '3', ordinal: 0, hour: 24, readHour: 26,
  countryTag: 'mod country', countryName: 'Old recorded name', currency: 'Political', category: 'advisors', itemId: 'advisor',
  chargeRaw: '15012345', realizedDeltaRaw: '-10000000', covered: true, occurrences: 1,
  changes: [{ readHour: 26, chargeRaw: '15012345', realizedDeltaRaw: '-10000000', covered: true, occurrences: 1 }], ...overrides });
const reply = (overrides: Partial<PoliticsLedgerReply> = {}): PoliticsLedgerReply => ({ branchId: '1', tag: 'mod country', decimals: 5,
  daily: [daily()], balanceChanges: [change()], warnings: [], spending: [spend()],
  spendItems: [{ id: 'advisor', groupId: 'advisor identity', name: 'Recorded advisor', action: 'Hired a recorded advisor', icon: null,
    tooltipToken: null, character: 7, targetTag: null, targetName: null, unknown: null }],
  spendCategories: [{ id: 'advisors', name: 'Advisors', identified: true }],
  spendingCoverage: [{ readHour: 24, sinceHour: 24, throughHour: 30, political: true, command: true, reason: null }],
  ideaTooltips: null, commandHolds: [], ...overrides });
const value = (rows: ReturnType<typeof powerBalanceRows>, label: string) => rows.find(row => row.label === label)?.value;

test('daily rates keep the game’s two decimals and payments are whole, without hour stamps or repeated changes', () => {
  const index = powerBalanceIndex(validatePowerBalance(reply()), 'Political');
  const rows = powerBalanceRows(index, 26, 26, 0);
  assert.equal(value(rows, 'Daily change'), '+1.49');
  assert.equal(value(rows, 'Gain'), '+2.66');
  assert.equal(value(rows, 'Daily costs'), '−1.17');
  assert.equal(rows.find(row => row.label === 'Gain')?.indent, true);
  assert.equal(value(rows, 'Hired a recorded advisor'), '−150');
  assert.equal(rows.find(row => row.label === 'Hired a recorded advisor')?.divider, true);
  assert.equal(rows.filter(row => row.label.includes('Hired')).length, 1, 'a payment is listed once');
  assert.equal(rows.some(row => /·|Adjustment|Daily income applied|Net daily income|Total held|missing/.test(row.label)), false);
  const precise = powerBalanceRows(index, 26, 26, 3);
  assert.equal(value(precise, 'Daily change'), '+1.490');
  assert.equal(value(precise, 'Hired a recorded advisor'), '−150.123');
});

test('a later observation never supplies income or a payment at an earlier hour', () => {
  const index = powerBalanceIndex(reply(), 'Political');
  for (const rows of [powerBalanceRows(index, 25, 30, 0), powerBalanceRows(index, 30, 25, 0)]) {
    assert.equal(value(rows, 'Daily change'), 'Not recorded');
    assert.equal(rows.some(row => row.label.includes('recorded advisor')), false);
  }
  assert.deepEqual(powerSpendTicks(index, [], 25), []);
  assert.deepEqual(powerSpendTicks(index, [], 26), [{ hour: 24 }]);
});

test('markers show one-off payments only, never costs charged every day', () => {
  const pp = powerBalanceIndex(reply(), 'Political');
  assert.deepEqual(powerSpendTicks(pp, [], 30), [{ hour: 24 }]);
  const recurring = powerBalanceIndex(reply({ spending: ['dailyCosts', 'organisationUpkeep'].map((category, at) => spend({ id: `daily ${at}`, category })),
    spendCategories: [{ id: 'dailyCosts', name: 'Daily costs', identified: true }, { id: 'organisationUpkeep', name: 'Organisation upkeep', identified: true }] }), 'Political');
  assert.deepEqual(powerSpendTicks(recurring, [], 30), [], 'daily costs would mark every day');
  assert.equal(powerBalanceRows(recurring, 26, 26, 0).some(row => /Daily costs:|Organisation upkeep/.test(row.label)), false,
    'they are part of the daily change, not listed again');
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
  const rows = powerBalanceRows(index, 26, 30, 0);
  assert.equal(value(rows, 'Daily change'), 'Not recorded');
  assert.ok(rows.some(row => row.label === 'Payments weren’t recorded at this time.'));
  assert.deepEqual(powerSpendTicks(index, [{ hour: 26, beforeRaw: '2', afterRaw: '1' }], 30), [{ hour: 26 }]);
});

test('daily rates never cross into another game day, including negative engine hours', () => {
  const data = reply({ daily: [daily({ dayHour: -1, readHour: -1 })], spending: [], balanceChanges: [] });
  const index = powerBalanceIndex(data, 'Political');
  assert.equal(value(powerBalanceRows(index, -1, -1, 0), 'Daily change'), '+1.49');
  assert.equal(value(powerBalanceRows(index, 0, 0, 0), 'Daily change'), 'Not recorded');
});

const hold = (readHour: number, throughHour: number, totalRaw: string | null): LedgerCommandHold => ({ readHour, throughHour, totalRaw,
  holders: [{ id: 'notIdentified', kind: 'Not identified', name: null, amountRaw: totalRaw }], reason: 'holder identity is not recorded' });

test('command power shows what moved into and out of reserve that day, never the counter as an amount held', () => {
  // The game's counter sits below zero: 12 reserved at hour 25, 20 released at 26.
  const holds = [hold(0, 24, '-7000000'), hold(25, 25, '-5800000'), hold(26, 30, '-7800000'), hold(32, 40, '-1000000')];
  const data = reply({ spending: [spend({ currency: 'Command', category: 'dailyDeficit', chargeRaw: '120001',
    changes: [{ readHour: 26, chargeRaw: '120001', realizedDeltaRaw: '-120001', covered: true, occurrences: 1 }] })],
  spendCategories: [{ id: 'dailyDeficit', name: 'Daily deficit', identified: true }], commandHolds: holds });
  const index = powerBalanceIndex(data, 'Command');
  const rows = powerBalanceRows(index, 26, 26, 0);
  assert.equal(value(rows, 'Daily deficit'), '−1.20');
  assert.equal(value(rows, 'Reserved'), '−12');
  assert.equal(value(rows, 'Released from reserve'), '+20');
  assert.equal(rows.some(row => /held|Reservation|unavailable|Not identified/.test(row.label)), false);
  assert.equal(value(powerBalanceRows(index, 25, 25, 0), 'Released from reserve'), undefined, 'a later release is not shown early');
  assert.deepEqual(reserveMoves(data, 24, 47), { reserved: 1200000n, released: 2000000n });
  assert.deepEqual(reserveMoves(data, 31, 47), { reserved: 0n, released: 0n }, 'a read gap is no move');
  assert.deepEqual(powerSpendTicks(index, [], 26), []);
});

test('gain the cap took is named, and period gain counts the game’s daily gain before costs', () => {
  const capped = reply({ daily: [daily({ ppIncomeRaw: '238400', ppGrossRaw: '238400', ppCostRaw: '0', ppRealizedRaw: '0',
    cpIncomeRaw: '50000', cpRealizedRaw: '0' })], spending: [], balanceChanges: [] });
  assert.equal(value(powerBalanceRows(powerBalanceIndex(capped, 'Political'), 26, 26, 0), 'Lost at the cap'), '−2.38');
  assert.deepEqual(powerGain([capped], 'Political', { from: null, hour: 26 }), { gained: 238400n, lost: 238400n, complete: true });
  assert.deepEqual(powerGain([capped], 'Command', { from: null, hour: 26 }), { gained: 50000n, lost: 50000n, complete: true });
  const days = reply({ daily: [daily(), daily({ dayHour: 48, readHour: 48 }), daily({ dayHour: 72, readHour: 72, ppGrossRaw: null })] });
  assert.deepEqual(powerGain([days], 'Political', { from: null, hour: 48 }), { gained: 532000n, lost: 0n, complete: true });
  assert.deepEqual(powerGain([days], 'Political', { from: 48, hour: 80 }), { gained: 266000n, lost: 0n, complete: false });
  assert.equal(powerGain([days], 'Political', { from: null, hour: 25 }), null, 'nothing read yet');
  assert.deepEqual(powerGain([days], 'Command', { from: null, hour: 48 }), { gained: 0n, lost: 0n, complete: true }, 'a deficit is spending, not negative gain');
  assert.deepEqual(powerGain([days], 'Political', { from: null, hour: 80 }, { matches: (_, hour) => hour !== 48 }),
    { gained: 266000n, lost: 0n, complete: false }, 'a group counts its members’ days only');
  const commandOnly = reply({ daily: [daily(), daily({ dayHour: 48, readHour: 50, ppGrossRaw: null, ppCostRaw: null, ppIncomeRaw: null,
    ppRealizedRaw: null, ppSequence: null })] });
  assert.deepEqual(powerGain([commandOnly], 'Political', { from: null, hour: 50 }), { gained: 266000n, lost: 0n, complete: false },
    'a day read without political power leaves its gain incomplete');
  assert.deepEqual(powerGain([commandOnly], 'Political', { from: null, hour: 49 }), { gained: 266000n, lost: 0n, complete: true },
    'a day not yet read is not missing');
});

test('malformed signed operands and future-dated application hours are refused', () => {
  for (const data of [reply({ daily: [daily({ ppGrossRaw: '1.2' })] }), reply({ daily: [daily({ readHour: 23 })] }),
    reply({ balanceChanges: [change({ readHour: 23 })] }), reply({ balanceChanges: [change({ realizedDeltaRaw: (1n << 127n).toString() })] })]) {
    assert.throws(() => validatePowerBalance(data));
  }
  assert.equal(validatePowerBalance(reply({ daily: [daily({ ppGrossRaw: '-9223372036854775808' })] })).daily![0].ppGrossRaw, '-9223372036854775808');
});

test('read-dated payments include only increments visible at the chosen hour', () => {
  const data = reply({ spending: [spend({ currency: 'Command', changes: [
    { readHour: 26, chargeRaw: '1', realizedDeltaRaw: '-1', covered: true, occurrences: 1 },
    { readHour: 28, chargeRaw: '2', realizedDeltaRaw: '-2', covered: true, occurrences: 1 },
  ] })] });
  const index = powerBalanceIndex(data, 'Command');
  assert.equal(value(powerBalanceRows(index, 26, 30, 5), 'Hired a recorded advisor'), '−0.00001');
  assert.equal(value(powerBalanceRows(index, 28, 30, 5), 'Hired a recorded advisor'), '−0.00003');
  assert.deepEqual(powerBalanceKeyHours(index), [26, 28]);
});

test('one daily currency arriving later cannot hide or backdate the other currency', () => {
  for (const ppFirst of [true, false]) {
    const data = validatePowerBalance(reply({ daily: [daily({ readHour: 25,
      ppReadHour: ppFirst ? 24 : 25, cpReadHour: ppFirst ? 25 : 24 })], spending: [], balanceChanges: [] }));
    const early = powerBalanceIndex(data, ppFirst ? 'Political' : 'Command');
    const later = powerBalanceIndex(data, ppFirst ? 'Command' : 'Political');
    assert.equal(value(powerBalanceRows(early, 24, 30, 0), ppFirst ? 'Daily change' : 'Daily deficit'), ppFirst ? '+1.49' : '−1.20');
    assert.equal(value(powerBalanceRows(later, 24, 30, 0), ppFirst ? 'Daily gain' : 'Daily change'), 'Not recorded');
    assert.notEqual(value(powerBalanceRows(later, 25, 30, 0), ppFirst ? 'Daily deficit' : 'Daily change'), 'Not recorded');
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
  assert.equal(value(powerBalanceRows(index, 26, 30, 0), 'Daily gain'), 'Not recorded');
  assert.deepEqual(powerBalanceKeyHours(index), []);
  for (const application of [{ cpRealizedRaw: '0' }, { cpSequence: '2' }]) {
    const known = powerBalanceIndex(reply({ daily: [{ ...absent, ...application }], balanceChanges: [], spending: [] }), 'Command');
    assert.equal(value(powerBalanceRows(known, 26, 30, 0), 'Daily gain'), 'Not recorded');
    assert.deepEqual(powerBalanceKeyHours(known), [26]);
  }
});

test('recorded balance changes never become their own tooltip rows, named or not', () => {
  const index = powerBalanceIndex(reply({ spending: [], balanceChanges: [change({ action: 'Assigned: Not identified', amountRaw: '0', realizedDeltaRaw: '0', reason: 'private' }),
    change({ action: 'Allocation: Not identified', amountRaw: '2000000', realizedDeltaRaw: '2000000', covered: false, reason: 'private' })] }), 'Political');
  const rows = powerBalanceRows(index, 26, 30, 0);
  assert.equal(rows.some(row => /Assigned|Allocation|Not identified|missing|private/.test(row.label)), false);
});

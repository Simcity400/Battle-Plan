import test from 'node:test';
import assert from 'node:assert/strict';
import { engineHour } from './calendar.ts';
import { groupAt, ledgerOf, monthFigures, powerGain, spendingGaps, spendingGroups, spendingItems, spendingTableRows, stableSpendingRows, validatePowerSpending } from './powerSpending.ts';
import { spendingChunks, structureAt, structureHours, SPENDING_CHUNK_HOURS } from './powerSpending.ts';
import { namesCovered, spendingCountryRequests, spendingRowsAhead } from './powerSpending.ts';
import type { PowerSpendingReply } from '../generated/dashboard';

const day = (month: number, date: number) => engineHour({ year: 1937, month, day: date, hour: 1 })!;
const item = (id: string, category: string, recurring: boolean, name: string | null) => ({ id, groupId: `g-${id}`, cause: recurring ? 'DailyGain' : 'Decision',
  category, recurring, firstHour: day(1, 1), firstRead: day(1, 1), name, action: name ?? 'Not identified', icon: null, character: null, targetTag: null,
  targetName: null, unknown: null, tooltipToken: null });
const categories = [{ id: 'decisions', name: 'Decisions', identified: true }, { id: 'dailyCosts', name: 'Daily costs', identified: true }];

function chunks(): PowerSpendingReply[] {
  const first: PowerSpendingReply = { branchId: '1', tag: 'GER', currency: 'Political', from: day(1, 1), to: day(1, 3) - 1, decimals: 5,
    items: [item('7:0:1', 'decisions', false, 'Recorded decision'), item('7:1:1', 'dailyCosts', true, 'Advisor cost')], categories, icons: [], ideaTooltips: null, warnings: [],
    payments: [{ item: 0, hour: day(1, 1) + 5, read: day(1, 1) + 6, sequence: '4', ordinal: 0, knownRaw: '5000000', payments: 1, unknown: 0, uncovered: false }],
    days: [{ item: 1, day: day(1, 1), read: day(1, 2), days: 1, chargeRaw: '200000', payments: 1, unknown: 0, uncovered: 0, realizedRaw: '-200000', credits: 0 }],
    gains: [{ hour: null, read: day(1, 2), gainedRaw: '300000', cappedRaw: '0', days: 1, unknownDays: 0, unmatched: 0 }],
    coverage: [{ firstHour: day(1, 1), lastHour: day(1, 1) + 1, covered: false, reason: 'Spending events were not recorded at this read' },
      { firstHour: day(1, 1) + 2, lastHour: day(1, 3) - 1, covered: true, reason: null }],
    gainCoverage: [{ firstHour: day(1, 1), lastHour: day(1, 3) - 1, covered: true, reason: null }],
    balances: [{ firstHour: day(1, 1), lastHour: day(1, 3) - 1, valueRaw: '0', reason: null }],
    reconciliation: { openingRaw: null, gainedRaw: null, lostAtCapRaw: null, spentRaw: null, allocatedRaw: null, releasedRaw: null, closingRaw: '0', otherChangesRaw: null, reason: 'Spending events were not recorded at this read' } };
  const second: PowerSpendingReply = { ...first, from: day(1, 3), to: day(4, 1),
    items: [item('7:1:1', 'dailyCosts', true, 'Advisor cost'), item('7:0:1', 'decisions', false, 'Recorded decision')],
    payments: [{ item: 1, hour: day(3, 2), read: day(3, 2), sequence: '9', ordinal: 0, knownRaw: '7000000', payments: 2, unknown: 1, uncovered: false }],
    days: [{ item: 0, day: day(1, 2), read: day(1, 3), days: 1, chargeRaw: '250000', payments: 1, unknown: 0, uncovered: 0, realizedRaw: null, credits: 0 }],
    gains: [{ hour: null, read: day(1, 3), gainedRaw: '300000', cappedRaw: '100000', days: 1, unknownDays: 1, unmatched: 0 }],
    coverage: [{ firstHour: day(1, 3), lastHour: day(4, 1), covered: true, reason: null }],
    gainCoverage: [{ firstHour: day(1, 3), lastHour: day(4, 1), covered: true, reason: null }],
    balances: [{ firstHour: day(1, 3) - 1, lastHour: day(4, 1), valueRaw: '0', reason: null }],
    reconciliation: { openingRaw: '0', gainedRaw: null, lostAtCapRaw: null, spentRaw: null, allocatedRaw: '0', releasedRaw: '0', closingRaw: '0', otherChangesRaw: null, reason: 'Some payment costs were not recorded' } };
  return [second, first].map(validatePowerSpending);
}

test('chunks join into one ledger: items by id, entries in read order, a coverage run cut by a chunk edge joined', () => {
  const ledger = ledgerOf('GER', 'Political', chunks());
  assert.deepEqual(ledger.payments.map(entry => [entry.read, entry.item.id, entry.known]), [[day(1, 1) + 6, '7:0:1', 5000000n], [day(3, 2), '7:0:1', 7000000n]]);
  assert.equal(ledger.payments[0].item, ledger.payments[1].item, 'one item object whichever chunk lists it');
  assert.deepEqual(ledger.days.map(entry => [entry.read, entry.charge]), [[day(1, 2), 200000n], [day(1, 3), 250000n]]);
  assert.deepEqual(ledger.coverage.map(run => [run.firstHour, run.lastHour, run.covered]), [[day(1, 1), day(1, 1) + 1, false], [day(1, 1) + 2, day(4, 1), true]]);
});

test('the table folds a recurring category into a month whose figures grow with each day read, and payments stay one row each', () => {
  const ledgers = [ledgerOf('GER', 'Political', chunks())];
  const mid = spendingTableRows(ledgers, { from: null, hour: day(1, 2) }, '');
  const month = mid.find(row => row.month !== null)!;
  assert.equal(month.days, 1);
  assert.equal(month.amount, 200000n);
  assert.deepEqual(monthFigures(month, day(1, 3)), { amount: 450000n, days: 2, partial: false, latest: month.monthDays[1] }, 'the next day is painted, not a new row');
  const end = spendingTableRows(ledgers, { from: null, hour: day(4, 1) }, '');
  const payments = end.filter(row => row.month === null);
  assert.deepEqual(payments.map(row => [row.amount, row.partial]), [[5000000n, false], [7000000n, true]], 'a payment whose charge was not recorded is partial, never guessed');
  assert.deepEqual(spendingTableRows(ledgers, { from: null, hour: day(4, 1) }, 'dailyCosts').map(row => row.category), ['dailyCosts']);
  assert.deepEqual(spendingTableRows(ledgers, { from: day(2, 1), hour: day(4, 1) }, '').map(row => row.amount), [7000000n], 'a range lists what was paid in it');
});

test('a block made again keeps each row that draws the same, so the table renders only new and changed rows', () => {
  const ledgers = [ledgerOf('GER', 'Political', chunks())];
  const before = spendingTableRows(ledgers, { from: null, hour: day(4, 1) }, '');
  const again = stableSpendingRows(new Map(before.map(row => [row.key, row])), spendingTableRows(ledgers, { from: null, hour: day(4, 1) }, ''));
  assert.ok(again.every((row, at) => row === before[at]), 'the same block: every row the very object drawn before');
  const earlier = spendingTableRows(ledgers, { from: null, hour: day(1, 2) }, '');
  const later = stableSpendingRows(new Map(earlier.map(row => [row.key, row])), spendingTableRows(ledgers, { from: null, hour: day(4, 1) }, ''));
  const month = later.find(row => row.month !== null)!;
  assert.notEqual(month, earlier.find(row => row.key === month.key), 'a month that read more days is a new row');
  assert.ok(later.filter(row => !earlier.some(held => held.key === row.key)).length > 0, 'payments the later block holds are new rows');
});

test('the breakdown reads each group at any hour from its cumulated entries, and gains and gaps name what is unknown', () => {
  const ledgers = [ledgerOf('GER', 'Political', chunks())];
  const groups = spendingGroups(ledgers, null);
  assert.deepEqual(groupAt(groups.get('g-7:0:1')!, day(2, 1)), { charge: 5000000n, payments: 1, unknown: 0, uncovered: false, latestHour: day(1, 1) + 5 });
  assert.equal(groupAt(groups.get('g-7:0:1')!, day(1, 1)), null);
  const items = spendingItems(groups, day(1, 2), day(4, 1));
  const decision = items.find(entry => entry.path[0].id === 'decisions')!;
  assert.equal(decision.exact, 5000000n);
  assert.equal(decision.final, 12000000);
  assert.deepEqual(powerGain(ledgers, { from: null, hour: day(1, 3) }), { gained: 600000n, lost: 100000n, complete: false });
  assert.equal(powerGain(ledgers, { from: null, hour: day(1, 1) }), null, 'no day counted: no gain, not zero');
  assert.deepEqual(spendingGaps(ledgers, { from: null, hour: day(4, 1) }).map(gap => [gap.from, gap.through]), [[day(1, 1), day(1, 1) + 1]]);
});


test('chunks are aligned, and the structure a head shows changes only where a payment, a month or a coverage run starts', () => {
  assert.deepEqual(spendingChunks(10, SPENDING_CHUNK_HOURS + 5), [[10, SPENDING_CHUNK_HOURS - 1], [SPENDING_CHUNK_HOURS, SPENDING_CHUNK_HOURS + 5]]);
  const hours = structureHours([ledgerOf('GER', 'Political', chunks())]);
  assert.equal(structureAt(hours, day(1, 2)), structureAt(hours, day(1, 3) - 1));
  assert.equal(structureAt(hours, day(1, 3)), structureAt(hours, day(1, 2)), 'a second day of the same month is painted, not structure');
  assert.ok(structureAt(hours, day(3, 2)) > structureAt(hours, day(3, 1)));
});


test('the rows a block draws ahead are asked their names with it, and rows reported mounted ask again only for a name not asked', () => {
  const rows = spendingTableRows([ledgerOf('GER', 'Political', chunks())], { from: null, hour: day(4, 1) }, '');
  const payment = rows.find(row => row.payment)!, month = rows.find(row => row.month !== null)!;
  assert.deepEqual(spendingRowsAhead(rows, payment.payment!.read, payment.payment!.read).map(row => row.key), [payment.key], 'a payment by its read');
  assert.ok(spendingRowsAhead(rows, month.hour, month.hour).some(row => row.key === month.key), 'a month by its latest day');
  const target = { ...payment, payment: { ...payment.payment!, item: { ...payment.payment!.item, targetTag: 'POL' } } };
  const ahead = spendingCountryRequests([], false, [target]);
  assert.deepEqual(ahead.wanted, [{ hour: target.hour, tag: 'POL' }]);
  assert.deepEqual([...new Set(spendingCountryRequests([month], true, [target]).wanted.map(item => item.tag))].sort(), ['GER', 'POL']);
  assert.ok(namesCovered(ahead.wanted, spendingCountryRequests([target], false).wanted), 'the names asked with a block cover its rows once mounted');
  assert.ok(!namesCovered(ahead.wanted, spendingCountryRequests([target], true).wanted), 'a country compared at its row is another name');
});

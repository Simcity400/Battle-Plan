import assert from 'node:assert/strict';
import test from 'node:test';
import { BANDS, dockyardItems, dockyardMonth, dockyardTotal, formatExactIc, monthColumns, monthsOf, nearestInteger, rateSum, repairAmount, repairItems, repairsRead, shownMonthBands, validateDockyardSpendingReply, visibleMonths } from './dockyardData.ts';
import { sunburstTree, unknownTotal } from '../../charts/sunburst/layout.ts';
import { mergeSpending } from '../production-spending/spendingData.ts';
import { firstHourOfMonth } from '../../dashboard/calendar.ts';
import type { ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun } from '../../generated/dashboard';

const purposeSum = (items: readonly { path: readonly { id: string }[]; exact?: bigint | null }[], id: string) =>
  items.filter(item => item.path[0].id === id).reduce((sum, item) => sum + (item.exact ?? 0n), 0n);
const known = (firstHour: number, lastHour: number, totalRaw: string): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw, reason: null });
const unknown = (firstHour: number, lastHour: number): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw: null, reason: 'spendingUnavailable' });
const leaf = (extra: Partial<ProductionSpendingLeaf>): ProductionSpendingLeaf => ({
  variant: '11', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', rocket: false, purpose: 'new', unproven: [],
  names: [{ firstHour: 100, name: 'Sea Wolf', known: true, design: null }],
  runs: [known(100, 123, '0'), known(124, 171, '3000000')], ...extra,
});

const reply = (): ProductionSpendingReply => ({
  branchId: '1', tag: 'opaque/tag', decimals: 5, reads: [100, 124, 148], observations: [100], warnings: [],
  equipmentReads: [100, 124, 148], rocketReads: [100], equipmentObservations: [100], rocketObservations: [100],
  rocketKey: null, rocketName: null, rocketTotal: [known(100, 171, '0')],
  production: [{ firstHour: 100, lastHour: 171, present: true, emptyCensusHour: null }],
  leaves: [
    leaf({}),
    leaf({ purpose: 'refit', runs: [known(100, 147, '0'), known(148, 171, '500000')] }),
    leaf({ variant: '12', typeKey: 'mod_hull_b', name: 'Hull B', names: [], runs: [known(100, 171, '0')] }),
    leaf({ variant: '13', typeKey: 'mod_rifle', name: 'Rifles', domain: 'land', runs: [known(100, 171, '9')] }),
  ],
  total: [known(100, 123, '0'), known(124, 147, '3000009'), known(148, 171, '3500009')],
  totalWithoutRefits: [known(100, 171, '0')],
  equipmentTotal: [known(100, 123, '0'), known(124, 147, '3000009'), known(148, 171, '3500009')],
  equipmentTotalWithoutRefits: [known(100, 171, '0')],
  repairs: { denominator: 24, total: [known(100, 171, '240000')], parts: [
    { variant: '11', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', names: [], runs: [known(100, 135, '100000'), unknown(136, 171)] },
    { variant: null, typeKey: '', name: null, domain: null, names: [], runs: [known(100, 135, '140000'), unknown(136, 171)] },
  ] },
  window: 0, firstHour: 100, lastHour: 171,
});

test('a dockyard reply needs its repair rates', () => {
  const good = reply();
  assert.equal(validateDockyardSpendingReply(good), good);
  assert.throws(() => validateDockyardSpendingReply({ ...good, repairs: null }), /malformed/);
  assert.ok(repairsRead(good));
});

test('a rate summed over hours is exact over a day’s hours, and unknown where an hour has no known rate', () => {
  const runs = [known(10, 19, '240'), known(20, 29, '480'), unknown(30, 34), known(35, 40, '24')];
  assert.deepEqual(rateSum(runs, 10, 19, 24), { exact: 2400n, reason: null, denominator: 24n });
  assert.deepEqual(rateSum(runs, 15, 24, 24), { exact: 5n * 240n + 5n * 480n, reason: null, denominator: 24n });
  assert.deepEqual(rateSum(runs, 25, 36, 24), { exact: null, reason: 'spendingUnavailable' });
  assert.deepEqual(rateSum(runs, 5, 12, 24), { exact: null, reason: 'outsideRead' });
  assert.deepEqual(rateSum([known(10, 19, '240'), known(25, 29, '240')], 15, 26, 24), { exact: null, reason: 'outsideRead' });
});

test('new ships and refits group by type and design beside repairs, land designs left out', () => {
  const data = reply();
  const items = dockyardItems(data, { from: null, hour: 160, end: 171 });
  assert.equal(purposeSum(items, 'new'), 3000000n);
  assert.equal(purposeSum(items, 'refit'), 500000n);
  assert.ok(!items.some(item => item.path.some(key => key.id === 'mod_rifle')));
  const repairs = items.filter(item => item.path[0].id === 'repair');
  const total = repairs.reduce((sum, item) => sum + (item.exact ?? 0n), 0n);
  assert.equal(total, 61n * 240000n);
  assert.equal(repairs.find(item => item.path.at(-1)?.id === '11')?.exact, null);
  assert.equal(repairs.find(item => item.path.at(-1)?.id === 'unattributed')?.exact, 61n * 240000n);
  const early = repairItems(data, { from: null, hour: 110, end: 120 });
  assert.deepEqual(early.map(item => item.exact), [11n * 100000n, 11n * 140000n]);
});

test('months are the calendar’s, clipped, each with its purposes and repairs, and a month not known says why', () => {
  const start = firstHourOfMonth(1936 * 12);
  assert.deepEqual(monthsOf(start + 5, start + 31 * 24 + 10), [{ firstHour: start + 5, lastHour: start + 31 * 24 - 1 }, { firstHour: start + 31 * 24, lastHour: start + 31 * 24 + 10 }]);
  const data = reply();
  const month = dockyardMonth(data, 100, 171, 100);
  assert.deepEqual(month.bands.map(band => band.exact), [3000000n, 500000n, 72n * 240000n, 0n, 0n]);
  assert.deepEqual(month.total, { exact: (3000000n + 500000n) * 24n + 72n * 240000n, reason: null, denominator: 24n });
  assert.deepEqual(monthColumns([month], 5)[0].values, [30, 5, 7.2, 0, 0]);
  assert.equal(dockyardMonth(data, 148, 171, 100).bands[1].exact, 500000n);
  data.equipmentTotal[2] = unknown(148, 171);
  assert.deepEqual(dockyardMonth(data, 148, 171, 100).bands[0], { exact: null, reason: 'spendingUnavailable' });
  const other = reply();
  other.leaves.push(leaf({ variant: '14', typeKey: ':mod_odd', name: null, domain: null, runs: [known(100, 147, '0'), known(148, 171, '7')] }));
  const marked = dockyardMonth(other, 148, 171, 100);
  assert.equal(marked.total.exact, null);
  assert.equal(marked.uncertain[0], true);
});

test('dockyard IC is rounded once from the exact amount', () => {
  assert.equal(nearestInteger(25n, 10n), 3n);
  assert.equal(nearestInteger(-25n, 10n), -3n);
  assert.equal(formatExactIc(61n * 240000n, 24n, 5, 2), '6.1');
});

test('all-unknown months become unknown columns without value height, and empty remainder bands stay out of the legend', () => {
  const data = reply();
  const knownMonth = dockyardMonth(data, 100, 171, 100);
  assert.deepEqual(shownMonthBands(BANDS, [knownMonth]).map(entry => entry.band.id), ['new', 'refit', 'repair']);
  const missing = { ...data, equipmentTotal: [unknown(100, 171)], repairs: { ...data.repairs!, total: [unknown(100, 171)] } };
  const missingMonth = dockyardMonth(missing, 100, 171, 100);
  const [column] = monthColumns([missingMonth], 5);
  assert.equal(column.unknown, true);
  assert.deepEqual(column.values, []);
  const withUnknownPurpose = { ...data, leaves: [...data.leaves, leaf({ variant: '99', purpose: 'unknown', runs: [known(100, 171, '50000')] })] };
  assert.deepEqual(shownMonthBands(BANDS, [dockyardMonth(withUnknownPurpose, 100, 171, 100)]).map(entry => entry.band.id),
    ['new', 'refit', 'repair', 'purpose-unrecorded']);
});

test('unknown repair split in an earlier window keeps a later ship withheld instead of inventing zero earlier repairs', () => {
  const base = reply();
  const win0 = validateDockyardSpendingReply({
    ...base, window: 0, firstHour: 0, lastHour: 2879, reads: [0], observations: [0], equipmentReads: [0], rocketReads: [0],
    equipmentObservations: [0], rocketObservations: [0], rocketTotal: [known(0, 2879, '0')],
    production: [{ firstHour: 0, lastHour: 2879, present: true, emptyCensusHour: null }], leaves: [],
    total: [known(0, 2879, '0')], totalWithoutRefits: [known(0, 2879, '0')],
    equipmentTotal: [known(0, 2879, '0')], equipmentTotalWithoutRefits: [known(0, 2879, '0')],
    repairs: { denominator: 24, total: [known(0, 2879, '24000')], parts: [
      { variant: null, typeKey: '', name: null, domain: null, names: [], runs: [unknown(0, 2879)] },
    ] },
  });
  const win1 = validateDockyardSpendingReply({
    ...win0, window: 1, firstHour: 2880, lastHour: 5759, reads: [2880], equipmentReads: [2880], rocketReads: [2880],
    rocketTotal: [known(2880, 5759, '0')], production: [{ firstHour: 2880, lastHour: 5759, present: true, emptyCensusHour: null }],
    total: [known(2880, 5759, '0')], totalWithoutRefits: [known(2880, 5759, '0')],
    equipmentTotal: [known(2880, 5759, '0')], equipmentTotalWithoutRefits: [known(2880, 5759, '0')],
    repairs: { denominator: 24, total: [known(2880, 2880, '2400000'), known(2881, 5759, '0')], parts: [
      { variant: 'D', typeKey: 'mod_hull_d', name: 'Hull D', domain: 'naval', names: [], runs: [known(2880, 2880, '2400000'), known(2881, 5759, '0')] },
      { variant: null, typeKey: '', name: null, domain: null, names: [], runs: [known(2880, 5759, '0')] },
    ] },
  });
  const merged = mergeSpending(new Map([[0, win0], [1, win1]]), 3000)!;
  const shipD = merged.repairs!.parts.find(part => part.variant === 'D')!;
  assert.deepEqual(repairAmount(merged, shipD.runs, { from: null, hour: 3000, end: 3000 }, 3000), { exact: null, reason: 'spendingUnavailable' });
  const items = repairItems(merged, { from: null, hour: 3000, end: 3000 });
  assert.equal(items.find(item => item.path.at(-1)?.id === 'D')?.exact, null);
});

test('a future repair reading never turns an unread past into zero', () => {
  const base = reply();
  const notRec = (firstHour: number, lastHour: number): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw: null, reason: 'notRecorded' });
  const win0 = validateDockyardSpendingReply({
    ...base, window: 0, firstHour: 0, lastHour: 2879, reads: [0], observations: [0], equipmentReads: [0], rocketReads: [0],
    equipmentObservations: [0], rocketObservations: [0], rocketTotal: [known(0, 2879, '0')],
    production: [{ firstHour: 0, lastHour: 2879, present: true, emptyCensusHour: null }], leaves: [],
    total: [known(0, 2879, '0')], totalWithoutRefits: [known(0, 2879, '0')],
    equipmentTotal: [known(0, 2879, '0')], equipmentTotalWithoutRefits: [known(0, 2879, '0')],
    repairs: { denominator: 24, total: [notRec(0, 2879)], parts: [{ variant: null, typeKey: '', name: null, domain: null, names: [], runs: [notRec(0, 2879)] }] },
  });
  const win3 = validateDockyardSpendingReply({
    ...win0, window: 3, firstHour: 8640, lastHour: 11519, reads: [8640], equipmentReads: [8640], rocketReads: [8640],
    rocketTotal: [known(8640, 11519, '0')], production: [{ firstHour: 8640, lastHour: 11519, present: true, emptyCensusHour: null }],
    total: [known(8640, 11519, '0')], totalWithoutRefits: [known(8640, 11519, '0')],
    equipmentTotal: [known(8640, 11519, '0')], equipmentTotalWithoutRefits: [known(8640, 11519, '0')],
    repairs: { denominator: 24, total: [known(8640, 11519, '24000')], parts: [{ variant: null, typeKey: '', name: null, domain: null, names: [], runs: [known(8640, 11519, '24000')] }] },
  });
  const atPast = mergeSpending(new Map([[0, win0], [3, win3]]), 500)!;
  assert.deepEqual(repairAmount(atPast, atPast.repairs!.total, { from: null, hour: 500, end: 500 }, 500), { exact: null, reason: 'notRecorded' });
  const both = mergeSpending(new Map([[0, win0], [3, win3]]))!;
  assert.deepEqual(repairAmount(both, both.repairs!.total, { from: null, hour: 500, end: 500 }, 500), { exact: null, reason: 'notRecorded' });
});

test('by-month visible bands use completed past months plus the current month through the playhead', () => {
  const data = validateDockyardSpendingReply({
    ...reply(),
    leaves: [
      leaf({ variant: '11', purpose: 'new', runs: [known(100, 171, '1000000')] }),
      leaf({ variant: '15', purpose: 'converted', runs: [known(100, 123, '0'), known(124, 147, '500000'), known(148, 171, '0')] }),
    ],
    total: [known(100, 123, '1000000'), known(124, 147, '1500000'), known(148, 171, '1000000')],
    equipmentTotal: [known(100, 123, '1000000'), known(124, 147, '1500000'), known(148, 171, '1000000')],
  });
  const fullMonth = dockyardMonth(data, 100, 171, 100);
  const through = (_index: number, hour: number) => dockyardMonth(data, 100, hour, 100);
  const atMid = visibleMonths([fullMonth], 135, through);
  assert.ok(shownMonthBands(BANDS, atMid).some(entry => entry.band.id === 'other-purpose'), '5 IC at playhead stays shown even when month-end corrects back to 0');
  const atEarly = visibleMonths([fullMonth], 110, through);
  assert.ok(!shownMonthBands(BANDS, atEarly).some(entry => entry.band.id === 'other-purpose'), 'purpose appearing only after playhead stays hidden');
});

test('partial repair coverage around an unread window gives the root a partial recorded total rather than hiding 240 IC', () => {
  const notRec = (firstHour: number, lastHour: number): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw: null, reason: 'notRecorded' });
  const mkWin = (window: number, firstHour: number, lastHour: number, leaves: ProductionSpendingLeaf[], repairRate: string | null): ProductionSpendingReply =>
    validateDockyardSpendingReply({
      ...reply(), window, firstHour, lastHour, reads: [firstHour], observations: [0], equipmentReads: [firstHour], rocketReads: [firstHour],
      equipmentObservations: [0], rocketObservations: [0], rocketTotal: [known(firstHour, lastHour, '0')],
      production: [{ firstHour, lastHour, present: true, emptyCensusHour: null }], leaves,
      total: [known(firstHour, lastHour, '3000000')], totalWithoutRefits: [known(firstHour, lastHour, '3000000')],
      equipmentTotal: [known(firstHour, lastHour, '3000000')], equipmentTotalWithoutRefits: [known(firstHour, lastHour, '3000000')],
      repairs: {
        denominator: 24,
        total: [repairRate == null ? notRec(firstHour, lastHour) : known(firstHour, lastHour, repairRate)],
        parts: [{ variant: null, typeKey: '', name: null, domain: null, names: [], runs: [repairRate == null ? notRec(firstHour, lastHour) : known(firstHour, lastHour, repairRate)] }],
      },
    });
  const shipA = (firstHour: number, lastHour: number) => leaf({ variant: 'A', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', purpose: 'new', runs: [known(firstHour, lastHour, '3000000')] });
  const shipB = (firstHour: number, lastHour: number) => leaf({ variant: 'B', typeKey: 'mod_hull_b', name: 'Hull B', domain: 'naval', purpose: 'new', runs: [known(firstHour, lastHour, '500000')] });
  const rateFor120Ic = String(100_000);
  const windows = new Map([
    [0, mkWin(0, 0, 2879, [shipA(0, 2879)], rateFor120Ic)],
    [1, mkWin(1, 2880, 5759, [shipA(2880, 5759)], null)],
    [2, mkWin(2, 5760, 8639, [shipA(5760, 8639)], rateFor120Ic)],
    [4, mkWin(4, 11520, 14399, [shipA(11520, 14399), shipB(11520, 14399)], rateFor120Ic)],
  ]);
  const merged = mergeSpending(windows, 8639)!;
  assert.ok(!merged.leaves.some(item => item.variant === 'B'), 'future window 4 is excluded at hour 8639');
  const win = { from: null, hour: 8639, end: 8639 };
  const items = dockyardItems(merged, win);
  const total = dockyardTotal(merged, win);
  assert.equal(total.unknownValue, false);
  assert.equal(total.exact, (30n + 240n) * 100_000n * 24n);
  assert.equal(total.exactDenominator, 24n);
  assert.ok(total.unknown && Object.values(total.unknown).reduce((a, b) => a + b, 0) > 0);
  const tree = sunburstTree(items, { id: 'all', label: 'All dockyard IC' }, undefined, { authoritativeTotal: total });
  assert.equal(tree.valueUnknown, false);
  assert.ok(unknownTotal(tree) > 0, 'root carries the partial flag');
});

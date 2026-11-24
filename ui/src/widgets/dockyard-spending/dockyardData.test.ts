import assert from 'node:assert/strict';
import test from 'node:test';
import { dockyardItems, formatExactIc, nearestInteger, rateSum, repairAmount, repairItems, repairsRead, repairsSince, validateDockyardSpendingReply } from './dockyardData.ts';
import { sunburstTree } from '../../charts/sunburst/layout.ts';
import { mergeSpending } from '../production-spending/spendingData.ts';
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

test('dockyard IC is rounded once from the exact amount', () => {
  assert.equal(nearestInteger(25n, 10n), 3n);
  assert.equal(nearestInteger(-25n, 10n), -3n);
  assert.equal(formatExactIc(61n * 240000n, 24n, 5, 2), '6.1');
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

test('a future repair date never leaks before the hour it appears, while archetype-classified ships keep their recorded type', () => {
  const notRec = (firstHour: number, lastHour: number): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw: null, reason: 'notRecorded' });
  const base = reply();
  const win1 = validateDockyardSpendingReply({
    ...base, window: 1, firstHour: 2880, lastHour: 5759, reads: [2880, 3000], observations: [2880],
    equipmentReads: [2880, 3000], rocketReads: [2880], equipmentObservations: [2880], rocketObservations: [2880],
    rocketTotal: [known(2880, 5759, '0')], production: [{ firstHour: 2880, lastHour: 5759, present: true, emptyCensusHour: null }],
    leaves: [leaf({ variant: '7', typeKey: 'mod_hull', name: 'Hulls', domain: 'naval', purpose: 'new',
      names: [{ firstHour: 3000, name: 'Type 34', design: null, known: true }], runs: [known(2880, 5759, '1000000')] })],
    total: [known(2880, 5759, '1000000')], totalWithoutRefits: [known(2880, 5759, '1000000')],
    equipmentTotal: [known(2880, 5759, '1000000')], equipmentTotalWithoutRefits: [known(2880, 5759, '1000000')],
    repairs: {
      denominator: 24,
      total: [notRec(2880, 2999), known(3000, 5759, '24000')],
      parts: [{ variant: null, typeKey: '', name: null, domain: null, names: [], runs: [notRec(2880, 2999), known(3000, 5759, '24000')] }],
    },
  });
  const windows = new Map([[1, win1]]);

  const at2890 = mergeSpending(windows, 2890)!;
  assert.equal(repairsSince(at2890, 2890), null);
  assert.equal(repairsRead(at2890, 2890), false);
  const items2890 = dockyardItems(at2890, { from: null, hour: 2890, end: 5759 });
  const tree2890 = sunburstTree(items2890, { id: 'all', label: 'All dockyard IC' });
  const new2890 = tree2890.children.find(child => child.key === 'new')!;
  assert.equal(new2890.valueUnknown, false);
  assert.equal(new2890.exact, 1000000n);
  const repair2890 = tree2890.children.find(child => child.key === 'repair')!;
  assert.equal(repair2890.valueUnknown, true);
  assert.ok(!repair2890.notes.some(note => note.includes('since ')), 'future repair start date must not appear at hour 2890');
  assert.ok(repair2890.notes.some(note => note.includes('Spending not recorded.')));
  assert.equal(tree2890.finalUnknown, true, 'Fill stays unknown while campaign end is in the future');

  const at3000 = mergeSpending(windows, 3000)!;
  assert.equal(repairsSince(at3000, 3000), 3000);
  assert.equal(repairsRead(at3000, 3000), true);
  const items3000 = dockyardItems(at3000, { from: null, hour: 3000, end: 5759 });
  const tree3000 = sunburstTree(items3000, { id: 'all', label: 'All dockyard IC' });
  const repair3000 = tree3000.children.find(child => child.key === 'repair')!;
  assert.equal(repair3000.valueUnknown, false);
  assert.equal(repair3000.exact, 1000n);
  assert.equal(repair3000.exactDenominator ?? 1n, 1n);
  assert.ok(repair3000.notes.some(note => note.includes('since ')));
});

test('withholding a future final keeps large purpose and repair designs shown instead of folded into Other', () => {
  const leaves: ProductionSpendingLeaf[] = Array.from({ length: 9 }, (_, index) => {
    const id = String(index + 1);
    return leaf({ variant: id, typeKey: `mod_h${id}`, name: `Hull ${id}`, domain: 'naval', purpose: 'new', names: [], runs: [known(100, 171, index === 8 ? '100000000' : '100000')] });
  });
  const repairParts = [
    ...Array.from({ length: 9 }, (_, index) => {
      const id = `r${index + 1}`;
      return { variant: id, typeKey: `mod_r${index + 1}`, name: `Repair ${index + 1}`, domain: 'naval' as const, names: [], runs: [known(100, 171, index === 8 ? '240000000' : '240000')] };
    }),
    { variant: null, typeKey: '', name: null, domain: null, names: [], runs: [known(100, 171, '0')] },
  ];
  const data = validateDockyardSpendingReply({
    ...reply(), leaves,
    total: [known(100, 171, '100800000')], equipmentTotal: [known(100, 171, '100800000')],
    repairs: { denominator: 24, total: [known(100, 171, '241920000')], parts: repairParts },
  });
  const tree = sunburstTree(dockyardItems(data, { from: null, hour: 130, end: 171 }), { id: 'all', label: 'All dockyard IC' });
  const newShips = tree.children.find(child => child.key === 'new')!;
  assert.ok(newShips.children.some(child => child.key === 'mod_h9' && !child.other), 'large naval design stays shown');
  assert.equal(newShips.children.find(child => child.key === 'mod_h9')?.finalUnknown, true);
  const repairs = tree.children.find(child => child.key === 'repair')!;
  assert.ok(repairs.children.some(child => child.key === 'mod_r9' && !child.other), 'large repair design stays shown');
  assert.equal(repairs.children.find(child => child.key === 'mod_r9')?.finalUnknown, true);
});

test('a repair gap withholds the incomplete repair total as on main so Breakdown and By month agree', () => {
  const data = validateDockyardSpendingReply({
    ...reply(),
    leaves: [leaf({ variant: '11', purpose: 'new', runs: [known(100, 102, '3000000')] })],
    total: [known(100, 102, '3000000')],
    equipmentTotal: [known(100, 102, '3000000')],
    repairs: {
      denominator: 24,
      total: [known(100, 100, '2400000'), unknown(101, 101), known(102, 102, '2400000')],
      parts: [
        { variant: 'A', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', names: [], runs: [known(100, 100, '2400000'), unknown(101, 101), known(102, 102, '2400000')] },
        { variant: null, typeKey: '', name: null, domain: null, names: [], runs: [known(100, 100, '0'), unknown(101, 101), known(102, 102, '0')] },
      ],
    },
    firstHour: 100, lastHour: 102,
  });
  const items = dockyardItems(data, { from: null, hour: 102, end: 102 });
  const repairs = items.filter(item => item.path[0].id === 'repair');
  assert.equal(repairs.find(item => item.path.at(-1)?.id === 'A')?.exact, null);
  assert.equal(repairs.find(item => item.path.at(-1)?.id === 'unattributed')?.exact, null);
});

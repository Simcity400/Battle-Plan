import assert from 'node:assert/strict';
import test from 'node:test';
import type { ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun } from '../../generated/dashboard';
import { amountIn, createWindowTracker, designAmount, displayedSpendingWindows, kindAmount, mergeSpending, noProductionAt, productionSpendingItems, selectSpendingState, spendingDesigns, spendingEmptyLabel, spendingFinals,
  spendingRecorded, spendingTotal, validateProductionSpendingReply, validateSpendingData } from './spendingData.ts';

const known = (firstHour: number, lastHour: number, totalRaw: string, breakBefore?: true): ProductionSpendingRun =>
  ({ firstHour, lastHour, totalRaw, reason: null, ...breakBefore ? { breakBefore } : {} });
const leaf = (extra: Partial<ProductionSpendingLeaf> = {}): ProductionSpendingLeaf => ({
  variant: '1', typeKey: 'mod_walker', name: 'Clockwork walkers', domain: 'land', rocket: false, purpose: 'new', unproven: [],
  names: [{ firstHour: 100, name: 'Early walker', known: true, design: null }],
  runs: [known(100, 123, '0'), known(124, 147, '1000'), known(148, 171, '3000')], ...extra,
});

const reply = (): ProductionSpendingReply => ({
  branchId: '1', tag: 'opaque/tag', decimals: 5, reads: [100, 124, 148], observations: [100], warnings: [],
  equipmentReads: [100, 124, 148], rocketReads: [100, 124, 148], equipmentObservations: [100], rocketObservations: [100],
  rocketKey: null, rocketName: null, rocketTotal: [known(100, 171, '0')],
  production: [{ firstHour: 100, lastHour: 123, present: false, emptyCensusHour: 100 }, { firstHour: 124, lastHour: 171, present: true, emptyCensusHour: null }],
  leaves: [
    leaf(),
    leaf({ purpose: 'converted', runs: [known(100, 147, '0'), known(148, 171, '500')] }),
    leaf({ variant: '2', typeKey: 'mod_hull', name: 'Hulls', domain: 'naval', purpose: 'new', names: [],
      runs: [known(100, 123, '0'), known(124, 171, '2000')] }),
    leaf({ variant: '2', typeKey: 'mod_hull', name: 'Hulls', domain: 'naval', purpose: 'refit', names: [],
      runs: [known(100, 147, '0'), known(148, 171, '700')] }),
  ],
  total: [known(100, 123, '0'), known(124, 147, '3000'), known(148, 171, '6200')],
  totalWithoutRefits: [known(100, 123, '0'), known(124, 147, '3000'), known(148, 171, '5500')],
  equipmentTotal: [known(100, 123, '0'), known(124, 147, '3000'), known(148, 171, '6200')],
  equipmentTotalWithoutRefits: [known(100, 123, '0'), known(124, 147, '3000'), known(148, 171, '5500')],
  repairs: null, window: 0, firstHour: 100, lastHour: 171,
});

test('a well-formed reply passes and a malformed one is refused', () => {
  const good = reply();
  assert.equal(validateProductionSpendingReply(good), good);
  const changes: ((bad: ProductionSpendingReply) => void)[] = [
    bad => { bad.decimals = 10; },
    bad => { bad.leaves[0].runs[1].totalRaw = '1.5'; },
    bad => { bad.leaves[0].runs[1].totalRaw = null; },
    bad => { bad.leaves[0].runs.reverse(); },
    bad => { bad.reads.reverse(); },
    bad => { bad.leaves.push(bad.leaves[0]); },
    bad => { bad.leaves[0].purpose = null; },
    bad => { bad.repairs = { denominator: 0, total: [], parts: [] }; },
  ];
  for (const change of changes) {
    const bad = structuredClone(good);
    change(bad);
    assert.throws(() => validateProductionSpendingReply(bad), /malformed/);
  }
});

test('a design adds the purposes counted, refits only when they are', () => {
  const data = reply();
  const withRefits = spendingDesigns(data, true);
  const without = spendingDesigns(data, false);
  assert.deepEqual(withRefits.map(design => [design.variant, design.parts.length]), [['1', 2], ['2', 2]]);
  assert.deepEqual(without.map(design => [design.variant, design.parts.length]), [['1', 2], ['2', 1]]);
  assert.equal(designAmount(data, withRefits[0], null, 160).exact, 3500n);
  assert.equal(designAmount(data, withRefits[1], null, 160).exact, 2700n);
  assert.equal(designAmount(data, without[1], null, 160).exact, 2000n);
  data.leaves[1].runs[1] = { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'lostOutput' };
  assert.deepEqual(designAmount(data, spendingDesigns(data, true)[0], null, 160), { exact: null, reason: 'lostOutput' });
});

test('a kind keeps its designs and its whole; a design of unknown arm leaves the kind not known', () => {
  const data = reply();
  assert.deepEqual(spendingDesigns(data, true, 'dockyards').map(design => design.variant), ['2']);
  const military = spendingDesigns(data, true, 'military');
  assert.equal(kindAmount(data, military, null, 160, 'military', true).exact, 3500n);
  assert.equal(kindAmount(data, spendingDesigns(data, true, 'dockyards'), 124, 160, 'dockyards', true).exact, 2700n);
  data.leaves.push(leaf({ variant: '9', typeKey: ':variant/9', name: null, domain: null, runs: [known(100, 147, '0'), known(148, 171, '10')] }));
  const both = spendingDesigns(data, true, 'military');
  assert.ok(spendingDesigns(data, true, 'dockyards').some(design => design.variant === '9'));
  assert.deepEqual(kindAmount(data, both, null, 160, 'military', true), { exact: null, reason: 'unattributed' });
  assert.equal(kindAmount(data, both, null, 130, 'military', true).exact, 1000n);
});

test('a span is measured within one observation and holder only', () => {
  const runs = [known(100, 123, '0'), known(124, 147, '1000'), known(148, 171, '50', true)];
  assert.equal(amountIn(runs, 124, 140, [100]).exact, 1000n);
  assert.deepEqual(amountIn(runs, 130, 160, [100]), { exact: null, reason: 'outsideRead' });
  assert.equal(amountIn(runs, 148, 160, [100, 148]).exact, 50n);
  assert.deepEqual(amountIn(runs, 120, 160, [100, 148]), { exact: null, reason: 'observationChanged' });
  assert.deepEqual(amountIn(runs, null, 200, [100]), { exact: null, reason: 'outsideRead' });
});

test('the items and the whole at an hour, and the empty census before any production', () => {
  const data = reply();
  const designs = spendingDesigns(data, false, 'military');
  const window = { from: null, hour: 130, end: 171 };
  const finals = spendingFinals(data, designs, window.from, window.end);
  assert.deepEqual(finals.map(final => final.exact), [3500n]);
  const items = productionSpendingItems(data, window, designs, finals, 'military', false);
  assert.deepEqual(items.map(item => [item.path.map(key => key.id), item.exact, item.present]), [[['land', 'mod_walker', '1'], 1000n, true]]);
  assert.equal(spendingTotal(data, designs, window, 'military', false).exact, 1000n);
  assert.ok(noProductionAt(data, 110));
  assert.equal(noProductionAt(data, 130), false);
  assert.equal(spendingEmptyLabel(data, designs, 110, null, 'military', false), 'No military production yet.');
  data.totalWithoutRefits[1] = { firstHour: 124, lastHour: 147, totalRaw: null, reason: 'missedPublication' };
  const unknown = productionSpendingItems(data, window, designs, finals, 'military', false);
  assert.deepEqual(unknown.at(-1)?.path, []);
  assert.equal(unknown.at(-1)?.unknownValue, true);
  assert.equal(spendingTotal(data, designs, window, 'military', false).note, 'Spending unavailable at this time.');
});

test('spending is recorded where the reply knows an amount, whether or not the output tables’ read hours were stamped', () => {
  const unstamped = { ...reply(), reads: [], equipmentReads: [], rocketReads: [],
    production: reply().production.map(run => ({ ...run, present: null, emptyCensusHour: null })) };
  assert.equal(validateProductionSpendingReply(unstamped), unstamped);
  assert.equal(spendingRecorded(unstamped), true, 'an older journal’s derived totals are spending recorded');
  assert.equal(noProductionAt(unstamped, 110), false, 'without a census, no hour is certified empty');
  const unknown = (firstHour: number, lastHour: number): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw: null, reason: 'notRecorded' });
  const none = { ...unstamped, total: [unknown(100, 171)], equipmentTotal: [unknown(100, 171)], rocketTotal: [unknown(100, 171)] };
  assert.equal(spendingRecorded(none), false);
});

function cut(whole: ProductionSpendingReply, window: number, first: number, last: number): ProductionSpendingReply {
  const runs = (items: readonly ProductionSpendingRun[]) => items.flatMap(run => {
    const from = Math.max(run.firstHour, first), to = Math.min(run.lastHour, last);
    if (from > to) return [];
    const { breakBefore, ...rest } = run;
    return [{ ...rest, firstHour: from, lastHour: to, ...breakBefore && from === run.firstHour ? { breakBefore } : {} }];
  });
  const hours = (items: readonly number[]) => items.filter(hour => first <= hour && hour <= last);
  return validateProductionSpendingReply({ ...whole, reads: hours(whole.reads), equipmentReads: hours(whole.equipmentReads), rocketReads: hours(whole.rocketReads),
    total: runs(whole.total), totalWithoutRefits: runs(whole.totalWithoutRefits), equipmentTotal: runs(whole.equipmentTotal),
    equipmentTotalWithoutRefits: runs(whole.equipmentTotalWithoutRefits), rocketTotal: runs(whole.rocketTotal),
    production: whole.production.flatMap(run => Math.max(run.firstHour, first) > Math.min(run.lastHour, last) ? []
      : [{ ...run, firstHour: Math.max(run.firstHour, first), lastHour: Math.min(run.lastHour, last) }]),
    leaves: whole.leaves.map(item => ({ ...item, runs: runs(item.runs), unproven: runs(item.unproven) })).filter(item => item.runs.length),
    window, firstHour: first, lastHour: last } as ProductionSpendingReply);
}

test('the windows held join into the reply of their hours together: every amount at every hour the same', () => {
  const whole = validateProductionSpendingReply(reply());
  const merged = mergeSpending(new Map([[1, cut(whole, 1, 140, 171)], [0, cut(whole, 0, 100, 139)]]));
  assert.ok(merged);
  assert.equal(validateSpendingData(merged), merged);
  assert.equal('window' in merged, false, 'a join names no one window');
  assert.deepEqual(merged.total, whole.total, 'runs across the edge are one run');
  assert.deepEqual(merged.leaves.map(item => item.runs), whole.leaves.map(item => item.runs));
  assert.deepEqual(merged.production, whole.production);
  assert.deepEqual(merged.reads, whole.reads);
  const designs = spendingDesigns(whole, true);
  for (const hour of [100, 130, 139, 140, 160, 171]) {
    for (const from of [null, 124, 141]) {
      assert.deepEqual(spendingTotal(merged, spendingDesigns(merged, true), { from, hour, end: 171 }, undefined, true),
        spendingTotal(whole, designs, { from, hour, end: 171 }, undefined, true), `${from}..${hour}`);
    }
  }
  assert.equal(mergeSpending(new Map()), null);
});

test('merging a later window fills omitted leaves as zero only in known hours and keeps unread leaves with their reason', () => {
  const early = cut(validateProductionSpendingReply(reply()), 0, 100, 139);
  const late = validateProductionSpendingReply({
    ...cut(validateProductionSpendingReply(reply()), 2, 160, 171),
    leaves: [
      ...cut(validateProductionSpendingReply(reply()), 2, 160, 171).leaves,
      leaf({ variant: '99', typeKey: 'mod_future', name: 'Future tank', domain: 'land', purpose: 'new', names: [], runs: [known(160, 171, '400')] }),
    ],
    total: [known(160, 171, '6600')],
    totalWithoutRefits: [known(160, 171, '5900')],
    equipmentTotal: [known(160, 171, '6600')],
    equipmentTotalWithoutRefits: [known(160, 171, '5900')],
  });
  const merged = mergeSpending(new Map([[0, early], [2, late]]))!;
  const military = spendingDesigns(merged, true, 'military');
  assert.equal(kindAmount(merged, military, null, 130, 'military', true).exact, 1000n, 'future design is zero in an earlier known window, never outsideRead');
  assert.equal(kindAmount(merged, military, null, 165, 'military', true).exact, 3900n);
  const unrecordedEarly = validateProductionSpendingReply({
    ...early,
    leaves: [],
    total: [{ firstHour: 100, lastHour: 139, totalRaw: null, reason: 'notRecorded' }],
    totalWithoutRefits: [{ firstHour: 100, lastHour: 139, totalRaw: null, reason: 'notRecorded' }],
    equipmentTotal: [{ firstHour: 100, lastHour: 139, totalRaw: null, reason: 'notRecorded' }],
    equipmentTotalWithoutRefits: [{ firstHour: 100, lastHour: 139, totalRaw: null, reason: 'notRecorded' }],
  });
  const withGap = mergeSpending(new Map([[0, unrecordedEarly], [2, late]]))!;
  const omitted = withGap.leaves.find(item => item.variant === '99')!;
  assert.deepEqual(amountIn(omitted.runs, null, 130, withGap.equipmentObservations), { exact: null, reason: 'notRecorded' });
});

test('a range starting on a 2,880-hour window boundary includes the preceding baseline window for exact amounts', () => {
  const base = reply();
  const win0 = validateProductionSpendingReply({
    ...base, window: 0, firstHour: 0, lastHour: 2879, reads: [0, 2879], observations: [0], equipmentReads: [0, 2879], rocketReads: [0],
    equipmentObservations: [0], rocketObservations: [0], rocketTotal: [known(0, 2879, '0')],
    production: [{ firstHour: 0, lastHour: 2879, present: true, emptyCensusHour: null }],
    leaves: [leaf({ variant: '2', typeKey: 'mod_hull', name: 'Hulls', domain: 'naval', purpose: 'new', names: [], runs: [known(0, 2879, '1000000')] })],
    total: [known(0, 2879, '1000000')], totalWithoutRefits: [known(0, 2879, '1000000')],
    equipmentTotal: [known(0, 2879, '1000000')], equipmentTotalWithoutRefits: [known(0, 2879, '1000000')],
    repairs: { denominator: 24, total: [known(0, 2879, '24000')], parts: [{ variant: null, typeKey: '', name: null, domain: null, names: [], runs: [known(0, 2879, '24000')] }] },
  });
  const win1 = validateProductionSpendingReply({
    ...win0, window: 1, firstHour: 2880, lastHour: 5759, reads: [2880, 3000], equipmentReads: [2880, 3000], rocketReads: [2880],
    rocketTotal: [known(2880, 5759, '0')], production: [{ firstHour: 2880, lastHour: 5759, present: true, emptyCensusHour: null }],
    leaves: [leaf({ variant: '2', typeKey: 'mod_hull', name: 'Hulls', domain: 'naval', purpose: 'new', names: [], runs: [known(2880, 5759, '1400000')] })],
    total: [known(2880, 5759, '1400000')], totalWithoutRefits: [known(2880, 5759, '1400000')],
    equipmentTotal: [known(2880, 5759, '1400000')], equipmentTotalWithoutRefits: [known(2880, 5759, '1400000')],
    repairs: { denominator: 24, total: [known(2880, 5759, '48000')], parts: [{ variant: null, typeKey: '', name: null, domain: null, names: [], runs: [known(2880, 5759, '48000')] }] },
  });
  const needed = displayedSpendingWindows({ firstHour: 0, lastHour: 5759 }, 3000, 2880, false, true);
  assert.deepEqual(needed, [0, 1]);
  const all = new Map([[0, win0], [1, win1]]);
  const merged = mergeSpending(new Map(needed.map(index => [index, all.get(index)!])), 3000)!;
  assert.equal(kindAmount(merged, spendingDesigns(merged, true, 'dockyards'), 2880, 3000, 'dockyards', true).exact, 400000n);
});

test('a design classification follows the displayed hour (not a later window-end definition), including on rewind', () => {
  const base = reply();
  const win0 = validateProductionSpendingReply({
    ...base, window: 0, firstHour: 0, lastHour: 2879, reads: [0], observations: [0], equipmentReads: [0], rocketReads: [0],
    equipmentObservations: [0], rocketObservations: [0], rocketTotal: [known(0, 2879, '0')],
    production: [{ firstHour: 0, lastHour: 2879, present: true, emptyCensusHour: null }],
    leaves: [leaf({ variant: '7', typeKey: ':variant/7', name: null, domain: null, purpose: 'new', names: [{ firstHour: 0, name: null, design: null, known: false }], runs: [known(0, 2879, '1000000')] })],
    total: [known(0, 2879, '1000000')], totalWithoutRefits: [known(0, 2879, '1000000')],
    equipmentTotal: [known(0, 2879, '1000000')], equipmentTotalWithoutRefits: [known(0, 2879, '1000000')],
  });
  const win1 = validateProductionSpendingReply({
    ...win0, window: 1, firstHour: 2880, lastHour: 5759, reads: [2880, 3000], equipmentReads: [2880, 3000], rocketReads: [2880],
    rocketTotal: [known(2880, 5759, '0')], production: [{ firstHour: 2880, lastHour: 5759, present: true, emptyCensusHour: null }],
    leaves: [leaf({ variant: '7', typeKey: 'mod_hull', name: 'Hulls', domain: 'naval', purpose: 'new',
      names: [{ firstHour: 3000, name: 'Type 34', design: null, known: true }], runs: [known(2880, 2999, '1000000'), known(3000, 5759, '3000000')] })],
    total: [known(2880, 2999, '1000000'), known(3000, 5759, '3000000')],
    totalWithoutRefits: [known(2880, 2999, '1000000'), known(3000, 5759, '3000000')],
    equipmentTotal: [known(2880, 2999, '1000000'), known(3000, 5759, '3000000')],
    equipmentTotalWithoutRefits: [known(2880, 2999, '1000000'), known(3000, 5759, '3000000')],
  });
  const held = new Map([[0, win0], [1, win1]]);
  const beforeDef = mergeSpending(held, 2890)!;
  assert.deepEqual(kindAmount(beforeDef, spendingDesigns(beforeDef, true, 'dockyards'), null, 2890, 'dockyards', true),
    { exact: null, reason: 'unattributed' });
  assert.equal(spendingEmptyLabel(beforeDef, spendingDesigns(beforeDef, true, 'dockyards'), 2890, null, 'dockyards', true), 'Equipment type unknown.');
  const atDef = mergeSpending(held, 3000)!;
  assert.deepEqual(kindAmount(atDef, spendingDesigns(atDef, true, 'dockyards'), null, 3000, 'dockyards', true),
    { exact: 3000000n, reason: null });
  const rewound = mergeSpending(held, 500)!;
  assert.deepEqual(kindAmount(rewound, spendingDesigns(rewound, true, 'dockyards'), null, 500, 'dockyards', true),
    { exact: null, reason: 'unattributed' });
  assert.equal(rewound.leaves[0].domain, null);
  assert.equal(rewound.leaves[0].typeKey, ':variant/7');
});

test('fill marks the final total unknown while the period end is in the future, and known once reached', () => {
  const data = validateProductionSpendingReply({
    ...reply(),
    leaves: [leaf({ variant: '1', typeKey: 'mod_walker', name: 'Walkers', domain: 'land', purpose: 'new',
      runs: [known(100, 147, '500000'), known(148, 171, '2000000')] })],
    total: [known(100, 147, '500000'), known(148, 171, '2000000')],
    totalWithoutRefits: [known(100, 147, '500000'), known(148, 171, '2000000')],
    equipmentTotal: [known(100, 147, '500000'), known(148, 171, '2000000')],
    equipmentTotalWithoutRefits: [known(100, 147, '500000'), known(148, 171, '2000000')],
  });
  const designs = spendingDesigns(data, true, 'military');
  const mid = { from: null, hour: 130, end: 171 };
  const midTotal = spendingTotal(data, designs, mid, 'military', true);
  assert.equal(midTotal.exact, 500000n);
  assert.equal(midTotal.finalUnknown, true);
  const midItems = productionSpendingItems(data, mid, designs, spendingFinals(data, designs, mid.from, mid.end, mid.hour), 'military', true);
  assert.equal(midItems[0].exact, 500000n);
  assert.equal(midItems[0].finalUnknown, true);

  const done = { from: null, hour: 171, end: 171 };
  const doneTotal = spendingTotal(data, designs, done, 'military', true);
  assert.equal(doneTotal.exact, 2000000n);
  assert.equal(doneTotal.finalUnknown, false);
  assert.equal(doneTotal.final, 2000000);
});

test('the actual frame window tracker reuses its result within a window and failed future windows never break a readable past', () => {
  const tracker = createWindowTracker();
  const branch = { firstHour: 0, lastHour: 14_399 };
  const first = tracker.get(branch, 100, null, false, true, true);
  assert.deepEqual(first, [0, 1]);
  const callsBefore = tracker.recomputations;
  for (let hour = 101; hour < 200; hour++) {
    assert.equal(tracker.get(branch, hour, null, false, true, true), first);
  }
  assert.equal(tracker.recomputations, callsBefore);
  const next = tracker.get(branch, 2880, null, false, true, true);
  assert.deepEqual(next, [0, 1, 2]);
  assert.equal(tracker.recomputations, callsBefore + 1);

  const win0 = cut(validateProductionSpendingReply(reply()), 0, 100, 171);
  const state = selectSpendingState([0], new Map([[0, win0]]), new Map([[1, { message: 'future failed', kind: 'error' as const }]]), 160);
  assert.equal(state.error, undefined);
  assert.ok(state.data);
  assert.equal(kindAmount(state.data, spendingDesigns(state.data, true, 'military'), null, 160, 'military', true).exact, 3500n);
});

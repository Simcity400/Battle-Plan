import assert from 'node:assert/strict';
import test from 'node:test';
import type { DockyardAmount, DockyardMonth, DockyardSpendingReply, ProductionSpendingLeaf, ProductionSpendingRun } from '../../generated/dashboard';
import { REPAIR_NOTE, dockyardItems, formatExactIc, monthColumns, monthLabel, monthNote, monthTotal, nearestInteger, reasonLabel, repairsRead, validateDockyardSpendingReply } from './dockyardData.ts';
import { definition, OPTIONS } from './definition.ts';
import { firstHourOfMonth } from '../../dashboard/calendar.ts';
import { groupTree } from '../../charts/sunburst/layout.ts';
import { unresolvedTypeName } from '../../dashboard/equipment.ts';

const purposeSum = (items: readonly { path: readonly { id: string }[]; exact?: bigint | null }[], id: string) =>
  items.filter(item => item.path[0].id === id).reduce((sum, item) => sum + (item.exact ?? 0n), 0n);
const known = (firstHour: number, lastHour: number, totalRaw: string): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw, reason: null });
const amount = (totalRaw: string | null): DockyardAmount => ({ totalRaw, reason: totalRaw == null ? 'recordingGap' : null });
const leaf = (extra: Partial<ProductionSpendingLeaf>): ProductionSpendingLeaf => ({
  variant: '11', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', rocket: false, purpose: 'new', unproven: [],
  names: [{ firstHour: 100, name: 'Sea Wolf', known: true, design: null }],
  runs: [known(100, 123, '0'), known(124, 171, '3000000')], ...extra,
});

const reply = (): DockyardSpendingReply => ({
  branchId: '1', tag: 'opaque/tag', decimals: 5, reads: [100, 124, 148], observations: [100], warnings: [],
  equipmentReads: [100, 124, 148], rocketReads: [100], equipmentObservations: [100], rocketObservations: [100],
  rocketKey: null, rocketName: null, rocketTotal: [known(100, 171, '0')],
  production: [{ firstHour: 100, lastHour: 171, present: true, emptyCensusHour: null }],
  leaves: [
    leaf({}),
    // The same design refitted to: its own leaf under Refits.
    leaf({ purpose: 'refit', runs: [known(100, 147, '0'), known(148, 171, '500000')] }),
    leaf({ variant: '12', typeKey: 'mod_hull_b', name: 'Hull B', names: [], runs: [known(100, 171, '0')] }),
  ],
  total: [known(100, 123, '0'), known(124, 147, '3000000'), known(148, 171, '3500000')],
  equipmentTotal: [known(100, 123, '0'), known(124, 147, '3000000'), known(148, 171, '3500000')],
  repairs: { hour: amount('250000'), end: amount('250000'), denominator: 24, since: null, parts: [] },
  months: [],
});

test('a dockyard reply needs purpose-split naval leaves, repairs and ordered months', () => {
  const good = reply();
  assert.equal(validateDockyardSpendingReply(good), good);
  const bad: ((value: DockyardSpendingReply) => void)[] = [
    value => { value.leaves[0].purpose = null; },
    value => { value.leaves[0].rocket = true; },
    value => { value.repairs.hour = { totalRaw: null, reason: null }; },
    value => { value.repairs.end = { totalRaw: '1.5', reason: null }; },
    value => { value.months = [month(0, 9), month(5, 19)]; },
    value => { value.leaves.push({ ...value.leaves[0] }); },
  ];
  for (const change of bad) {
    const value = reply();
    change(value);
    assert.throws(() => validateDockyardSpendingReply(value));
  }
  // One design new and refitted keeps both leaves.
  assert.doesNotThrow(() => validateDockyardSpendingReply(reply()));
});

test('new ships and refits group by type and design, and repairs are one unattributed part', () => {
  const items = dockyardItems(reply(), { from: null, hour: 171, end: 171 });
  const shown = items.filter(item => item.present);
  assert.deepEqual(shown.map(item => [item.path.map(part => part.label), item.exact]), [
    [['New ships', 'Hull A', 'Sea Wolf'], 3000000n],
    [['Refits', 'Hull A', 'Sea Wolf'], 500000n],
    [['Repairs', 'Unattributed'], 250000n],
  ]);
  assert.equal(shown[2].note, REPAIR_NOTE);
  // A range subtracts its baseline within one observation.
  const range = dockyardItems(reply(), { from: 148, hour: 171, end: 171 });
  assert.deepEqual(range.filter(item => item.present && item.path[0].id !== 'repair').map(item => item.exact), [500000n]);
});

test('an unresolved design or an unproven line marks attribution incomplete, and unknown repairs stay unknown', () => {
  const value = reply();
  value.leaves.push(leaf({ variant: '99', typeKey: ':mod_unknown', name: null, domain: null, names: [] }));
  const items = dockyardItems(value, { from: null, hour: 171, end: 171 });
  assert.ok(!items.some(item => item.path[2]?.id === '99'), 'it is left out');
  const newShip = items.find(item => item.path[0].id === 'new' && item.present)!;
  assert.deepEqual(newShip.unknownLevels, [true, false, false]);
  const refit = items.find(item => item.path[0].id === 'refit' && item.present)!;
  assert.deepEqual(refit.unknownLevels, [false, false, false], 'the unresolved design has no refit input');

  // Unrecorded-purpose input of the design from hour 124: 500000 by 148.
  const unproven = reply();
  unproven.leaves.push(leaf({ purpose: 'unknown', runs: [known(100, 123, '0'), known(124, 171, '500000')],
    unproven: [known(100, 123, '0'), known(124, 171, '500000')] }));
  unproven.total = unproven.equipmentTotal = [known(100, 123, '0'), known(124, 147, '3500000'), known(148, 171, '4000000')];
  const marked = dockyardItems(unproven, { from: null, hour: 171, end: 171 });
  const purposeParts = marked.filter(item => ['new', 'refit'].includes(item.path[0].id) && item.present);
  assert.ok(purposeParts.every(item => item.localUnknownLevels?.[0] && !item.unknownLevels?.[0]),
    'purpose uncertainty stays on New ships and Refits');
  // Before the input joined nothing is uncertain now, but the end still is.
  const early = dockyardItems(unproven, { from: null, hour: 110, end: 171 }).find(item => item.path[0].id === 'new')!;
  assert.equal(early.localUnknownLevels?.[0], false);
  assert.equal(early.localFinalUnknownLevels?.[0], true, 'the end reads its own unproven input');
  // A range after the input joined holds none of it.
  const later = dockyardItems(unproven, { from: 148, hour: 171, end: 171 }).find(item => item.path[0].id === 'refit')!;
  assert.equal(later.localUnknownLevels?.[0], false);
  // The whole stays known: purpose only splits it.
  const tree = groupTree(marked, { id: 'all', label: 'All' });
  assert.equal(tree.valueUnknown, false);
  // New, refit, unrecorded and the repairs (250000 over 24 hours), exactly.
  assert.equal(tree.exact! * 24n, (4_000_000n * 24n + 250_000n) * (tree.exactDenominator ?? 1n));
  const node = (key: string) => tree.children.find(child => child.key === key)!;
  assert.equal(node('new').valueUnknown, true);
  // New ships keep only recorded new input; the rest is its own part. Both
  // nodes are uncertain, so their designs carry the exact amounts.
  assert.equal(node('purpose-unrecorded').valueUnknown, true);
  assert.equal(purposeSum(marked, 'new'), 3_000_000n);
  assert.equal(purposeSum(marked, 'purpose-unrecorded'), 500_000n);

  const gap = reply();
  gap.repairs.hour = amount(null);
  const repair = dockyardItems(gap, { from: null, hour: 171, end: 171 }).find(item => item.path[0].id === 'repair')!;
  assert.equal(repair.unknownValue, true);
  assert.equal(repair.exact, null);
  assert.ok(repair.note?.startsWith(REPAIR_NOTE));
});

function month(firstHour: number, lastHour: number, extra: Partial<DockyardMonth> = {}): DockyardMonth {
  return { firstHour, lastHour, newShips: amount('100000'), refits: amount('-20000'), repairs: amount('5'),
    unrecorded: amount('0'), other: amount('0'), purposeUnproven: false, unresolved: false, ...extra };
}

test('months become columns in purpose order, and an unknown part leaves the total unknown', () => {
  const months = [month(0, 9), month(10, 19, { repairs: amount(null), unresolved: true })];
  // Repairs are numerators over the hours of a day.
  assert.deepEqual(monthColumns(months, 5, 24), [
    { firstHour: 0, lastHour: 9, values: [1, -0.2, 5 / 24 / 100_000, 0, 0], uncertain: [false, false, false, false, false] },
    { firstHour: 10, lastHour: 19, values: [1, -0.2, null, 0, 0], uncertain: [true, true, false, false, false] },
  ]);
  assert.deepEqual(monthTotal(months[0], 24), { exact: 80_000n * 24n + 5n, reason: null, denominator: 24n });
  // Unrecorded and other-purpose input are bands of their own in the whole.
  const remainders = month(0, 9, { unrecorded: amount('30000'), other: amount('2000'), purposeUnproven: true });
  assert.deepEqual(monthColumns([remainders], 5, 24)[0].uncertain, [true, true, false, true, false]);
  assert.deepEqual(monthTotal(remainders, 24), { exact: 112_000n * 24n + 5n, reason: null, denominator: 24n });
  assert.deepEqual(monthTotal(months[1], 24), { exact: null, reason: 'unresolved' });
  assert.deepEqual(monthTotal({ ...months[1], unresolved: false }, 24), { exact: null, reason: 'recordingGap' });
  assert.equal(nearestInteger(80_000n * 24n + 5n, 24n), 80_000n);
  assert.equal(nearestInteger(-25n, 10n), -3n);
  assert.equal(monthNote(months[0]), undefined);
  assert.ok(monthNote(months[1]));
  assert.equal(monthLabel(firstHourOfMonth(1937 * 12 + 2)), 'March 1937');
});

test('both views are settings and catalogue cards', () => {
  assert.equal(OPTIONS.find(field => field.key === 'view')?.default, 'circle');
  assert.deepEqual(definition.variants.map(variant => variant.initial), [{ title: 'Dockyard spending by month', options: { view: 'months' } }]);
  assert.deepEqual(definition.timeModes, ['campaign', 'follow', 'pin', 'range']);
});

test('repairs split into repaired designs by type, with the rest unattributed', () => {
  const value = reply();
  value.repairs.parts = [
    { variant: '11', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', names: [{ firstHour: 100, name: 'Sea Wolf', known: true, design: null }],
      hour: amount('150000'), end: amount('150000') },
    { variant: '30', typeKey: ':mod_old_hull', name: null, domain: null, names: [], hour: amount('0'), end: amount('40000') },
    { variant: null, typeKey: '', name: null, domain: null, names: [], hour: amount('100000'), end: amount('100000') },
  ];
  assert.equal(validateDockyardSpendingReply(value), value);
  const repairs = dockyardItems(value, { from: null, hour: 171, end: 171 }).filter(item => item.path[0].id === 'repair');
  assert.deepEqual(repairs.map(item => [item.path.map(part => part.label), item.exact, item.present]), [
    [['Repairs', 'Hull A', 'Sea Wolf'], 150000n, true],
    // Not repaired by the moment shown, but kept for its final order and colour.
    [['Repairs', unresolvedTypeName(':mod_old_hull'), '30'], 0n, false],
    [['Repairs', 'Unattributed'], 100000n, true],
  ]);
  assert.equal(repairs[1].final, 40000 / 24);
  assert.ok(repairs.every(item => item.note?.startsWith(REPAIR_NOTE)));
  // Each design appears once; a repeated part is refused.
  value.repairs.parts.push({ ...value.repairs.parts[0] });
  assert.throws(() => validateDockyardSpendingReply(value));
});

test('a refused split keeps the known repairs total, the rest unattributed and the split marked incomplete', () => {
  const value = reply();
  value.repairs = { hour: amount('300000'), end: amount('300000'), denominator: 1, since: null, parts: [
    { variant: '11', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', names: [], hour: amount('100000'), end: amount('100000') },
    { variant: '12', typeKey: 'mod_hull_b', name: 'Hull B', domain: 'naval', names: [], hour: amount(null), end: amount(null) },
    { variant: null, typeKey: '', name: null, domain: null, names: [], hour: amount(null), end: amount(null) },
  ] };
  const repairs = dockyardItems(value, { from: null, hour: 171, end: 171 }).filter(item => item.path[0].id === 'repair');
  assert.deepEqual(repairs.map(item => [item.path.at(-1)!.id, item.exact, item.unknownValue, item.present]), [
    ['11', 100000n, false, true],
    // Withdrawn: not measured, its amount is in the rest.
    ['12', null, false, false],
    ['unattributed', 200000n, false, true],
  ]);
  assert.ok(repairs.every(item => !item.finalUnknown));
  // The known parts and the rest add up to the known total.
  assert.equal(repairs.reduce((sum, item) => sum + (item.exact ?? 0n), 0n), 300000n);
  // Grouped, Repairs and the whole keep the known totals; only the types
  // that miss a design's share are uncertain.
  const tree = groupTree(repairs, { id: 'all', label: 'All' });
  const repairNode = tree.children.find(node => node.key === 'repair')!;
  assert.equal(tree.valueUnknown, false);
  assert.equal(tree.exact, 300000n);
  assert.equal(repairNode.valueUnknown, false);
  assert.equal(repairNode.exact, 300000n);
  assert.equal(repairNode.finalUnknown, false);
  const byKey = new Map(repairNode.children.map(node => [node.key, node]));
  assert.equal(byKey.get('mod_hull_a')!.valueUnknown, true);
  assert.equal(byKey.get('unattributed')!.valueUnknown, false);
  assert.equal(byKey.get('unattributed')!.exact, 200000n);
});

test('repairs counted from a later start name it, and spending only on unresolved designs stays present', () => {
  const late = reply();
  late.repairs.since = firstHourOfMonth(1937 * 12);
  const repair = dockyardItems(late, { from: null, hour: 171, end: 171 }).find(item => item.path[0].id === 'repair')!;
  assert.match(repair.note ?? '', /^Dockyard output used for repairs since .*1937\.$/);

  const unresolvedOnly = reply();
  unresolvedOnly.leaves = [leaf({ variant: '99', typeKey: ':mod_unknown', name: null, domain: null, names: [] })];
  unresolvedOnly.repairs = { hour: amount('0'), end: amount('0'), denominator: 24, since: null, parts: [] };
  const items = dockyardItems(unresolvedOnly, { from: null, hour: 171, end: 171 });
  const placeholder = items.filter(item => item.present);
  assert.deepEqual(placeholder.map(item => item.path.map(part => part.id)), [['new', 'unresolved']]);
  assert.deepEqual(placeholder[0].unknownLevels, [true, true, false]);
  assert.ok(placeholder[0].note);
});

test('repair parts and their total share one denominator, so an hour-long split adds exactly', () => {
  const value = reply();
  // Two designs repaired for one hour at one dockyard each: 100000 fixed5 a day.
  value.repairs = { hour: amount('200000'), end: amount('200000'), denominator: 24, since: null, parts: [
    { variant: '11', typeKey: 'mod_hull_a', name: 'Hull A', domain: 'naval', names: [], hour: amount('100000'), end: amount('100000') },
    { variant: '12', typeKey: 'mod_hull_b', name: 'Hull B', domain: 'naval', names: [], hour: amount('100000'), end: amount('100000') },
    { variant: null, typeKey: '', name: null, domain: null, names: [], hour: amount('0'), end: amount('0') },
  ] };
  const repairs = dockyardItems(value, { from: null, hour: 171, end: 171 }).filter(item => item.path[0].id === 'repair');
  const node = groupTree(repairs, { id: 'all', label: 'All' }).children[0];
  // 200000 / 24 fixed5, never 2 x floor(100000 / 24).
  assert.equal(node.exact! * 24n, 200_000n * (node.exactDenominator ?? 1n));
  assert.equal(nearestInteger(node.exact!, node.exactDenominator ?? 1n), 8_333n);
  assert.equal(repairs[0].value, 100_000 / 24);
});

test('dockyard IC is rounded once from the exact amount, in both views', () => {
  // 1199999 / 24 fixed5 is 0.4999996 IC: whole numbers write 0, never 1.
  assert.equal(formatExactIc(1_199_999n, 24n, 5, 0), '0');
  assert.equal(formatExactIc(-1_199_999n, 24n, 5, 0), '0');
  assert.equal(formatExactIc(1_200_000n, 24n, 5, 0), '1');
  assert.equal(formatExactIc(-1_200_000n, 24n, 5, 0), '−1');
  assert.equal(formatExactIc(1_199_999n, 24n, 5, 5), '0.5');
  assert.equal(formatExactIc(250_000n, 1n, 5, 1), '2.5');
  // A month's total, as its tooltip writes it.
  const total = monthTotal(month(0, 9, { newShips: amount('0'), refits: amount('0'), repairs: amount('1199999') }), 24);
  assert.equal(formatExactIc(total.exact!, total.denominator!, 5, 0), '0');
});

test('unrecorded-purpose input stays present and uncertain even where it nets to zero', () => {
  const value = reply();
  // Unrecorded input up and down by as much: its leaf nets to zero.
  value.leaves = [leaf({}), leaf({ purpose: 'unknown', runs: [known(100, 171, '0')],
    unproven: [known(100, 123, '0'), known(124, 171, '100000')] })];
  value.repairs = { hour: amount('0'), end: amount('0'), denominator: 24, since: null, parts: [] };
  const items = dockyardItems(value, { from: null, hour: 171, end: 171 });
  const present = items.filter(item => item.present);
  assert.deepEqual(present.map(item => item.path.map(part => part.id)).sort(),
    [['new', 'mod_hull_a', '11'], ['purpose-unrecorded', 'mod_hull_a', '11']].sort());
  const unrecorded = present.find(item => item.path[0].id === 'purpose-unrecorded')!;
  assert.deepEqual(unrecorded.localUnknownLevels, [true, true, true], 'uncertain down its subtree');
  const tree = groupTree(items, { id: 'all', label: 'All' });
  assert.equal(tree.valueUnknown, false, 'the whole stays known');
  assert.equal(tree.exact, 3_000_000n * (tree.exactDenominator ?? 1n));
  assert.equal(tree.children.find(node => node.key === 'new')?.valueUnknown, true);
  assert.equal(purposeSum(items, 'new'), 3_000_000n);
});

test('an unresolved design of a purpose naval production never has leaves the whole uncertain', () => {
  const value = reply();
  // 1 IC new and 2 IC of conversions of a design content cannot type.
  value.leaves = [leaf({ runs: [known(100, 171, '100000')] }),
    leaf({ variant: '99', typeKey: ':mod_unknown', name: null, domain: null, names: [], purpose: 'converted', runs: [known(100, 171, '200000')] })];
  value.repairs = { hour: amount('0'), end: amount('0'), denominator: 24, since: null, parts: [] };
  const items = dockyardItems(value, { from: null, hour: 171, end: 171 });
  const marker = items.find(item => item.path[0].id === 'other-purpose')!;
  assert.deepEqual(marker.path.map(part => part.id), ['other-purpose', 'unresolved']);
  assert.ok(marker.present && marker.unknownLevels?.[0]);
  const tree = groupTree(items, { id: 'all', label: 'All' });
  assert.equal(tree.valueUnknown, true, 'never an exact 1 IC');
  // Whichever rings are hidden, the whole stays uncertain.
  for (const shown of [[false, false, true], [false, true, false]]) {
    assert.equal(groupTree(items, { id: 'all', label: 'All' }, shown).valueUnknown, true, JSON.stringify(shown));
  }
});

test('naval input of a purpose naval production never has stays in the whole as Other purposes', () => {
  const value = reply();
  // 1 IC new and 2 IC recorded as conversions of a naval design.
  value.leaves = [leaf({ runs: [known(100, 171, '100000')] }), leaf({ purpose: 'converted', runs: [known(100, 171, '200000')] })];
  value.repairs = { hour: amount('0'), end: amount('0'), denominator: 24, since: null, parts: [] };
  const items = dockyardItems(value, { from: null, hour: 171, end: 171 });
  assert.deepEqual(items.filter(item => item.present).map(item => [item.path[0].label, item.exact]),
    [['New ships', 100_000n], ['Other purposes', 200_000n]]);
  const tree = groupTree(items, { id: 'all', label: 'All' });
  assert.equal(tree.exact, 300_000n * (tree.exactDenominator ?? 1n), 'the whole counts it');
});

test('unread equipment spending is unknown under both purposes, not hidden behind known repairs', () => {
  const value = reply();
  value.leaves = [];
  value.equipmentTotal = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'notRecorded' }];
  const items = dockyardItems(value, { from: null, hour: 171, end: 171 });
  const unread = items.filter(item => item.path[1]?.id === 'not-known');
  assert.deepEqual(unread.map(item => item.path[0].id), ['new', 'refit']);
  assert.ok(unread.every(item => item.present && item.unknownValue && item.finalUnknown));
  const tree = groupTree(items, { id: 'all', label: 'All' });
  assert.equal(tree.valueUnknown, true, 'repairs are not the whole');
  // Readable equipment adds no such part.
  assert.ok(!dockyardItems(reply(), { from: null, hour: 171, end: 171 }).some(item => item.path[1]?.id === 'not-known'));
});

test('a month with unresolved designs has no known total; an unrecorded purpose keeps it', () => {
  const unresolved = month(0, 9, { unresolved: true });
  assert.deepEqual(monthTotal(unresolved, 24), { exact: null, reason: 'unresolved' });
  assert.equal(reasonLabel('unresolved'), 'Some designs’ types are unknown.');
  const unproven = month(0, 9, { purposeUnproven: true });
  assert.notEqual(monthTotal(unproven, 24).exact, null);
});

test('known repairs are read without production reads', () => {
  const value = reply();
  value.reads = [];
  assert.equal(repairsRead(value), true);
  value.repairs.hour = amount(null);
  assert.equal(repairsRead(value), false);
  value.months = [month(0, 9)];
  assert.equal(repairsRead(value), true);
});

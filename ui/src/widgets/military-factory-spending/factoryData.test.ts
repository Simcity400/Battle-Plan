import assert from 'node:assert/strict';
import test from 'node:test';
import type { DockyardAmount, FactoryMonth, MilitaryFactorySpendingReply, ProductionSpendingLeaf, ProductionSpendingRun } from '../../generated/dashboard';
import { PURPOSES, factoryColumns, factoryItems, factoryMonthNote, factoryMonthTotal, validateFactorySpendingReply } from './factoryData.ts';
import { definition, OPTIONS } from './definition.ts';
import { groupTree } from '../../charts/sunburst/layout.ts';

const purposeSum = (items: readonly { path: readonly { id: string }[]; exact?: bigint | null }[], id: string) =>
  items.filter(item => item.path[0].id === id).reduce((sum, item) => sum + (item.exact ?? 0n), 0n);
const known = (firstHour: number, lastHour: number, totalRaw: string): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw, reason: null });
const amount = (totalRaw: string | null): DockyardAmount => ({ totalRaw, reason: totalRaw == null ? 'recordingGap' : null });
const leaf = (extra: Partial<ProductionSpendingLeaf>): ProductionSpendingLeaf => ({
  variant: '11', typeKey: 'mod_fighter', name: 'Fighter', domain: 'air', rocket: false, purpose: 'new', unproven: [],
  names: [{ firstHour: 100, name: 'Kestrel II', known: true, design: null }],
  runs: [known(100, 123, '0'), known(124, 171, '3000000')], ...extra,
});

const reply = (): MilitaryFactorySpendingReply => ({
  branchId: '1', tag: 'opaque/tag', decimals: 5, reads: [100, 124, 148], observations: [100], warnings: [],
  equipmentReads: [100, 124, 148], rocketReads: [100], equipmentObservations: [100], rocketObservations: [100],
  rocketKey: null, rocketName: null, rocketTotal: [known(100, 171, '0')],
  production: [{ firstHour: 100, lastHour: 171, present: true, emptyCensusHour: null }],
  leaves: [
    leaf({}),
    // The same design converted into: its own leaf under Converted.
    leaf({ purpose: 'converted', runs: [known(100, 147, '0'), known(148, 171, '700000')] }),
  ],
  total: [known(100, 123, '0'), known(124, 147, '3000000'), known(148, 171, '3700000')],
  equipmentTotal: [known(100, 123, '0'), known(124, 147, '3000000'), known(148, 171, '3700000')],
  months: [],
});

function month(firstHour: number, lastHour: number, extra: Partial<FactoryMonth> = {}): FactoryMonth {
  return { firstHour, lastHour, new: amount('100000'), converted: amount('-20000'), unrecorded: amount('0'), other: amount('0'),
    purposeUnproven: false, unresolved: false, ...extra };
}

test('a military factory reply needs new and converted equipment leaves and ordered months', () => {
  const good = reply();
  assert.equal(validateFactorySpendingReply(good), good);
  const bad: ((value: MilitaryFactorySpendingReply) => void)[] = [
    value => { value.leaves[0].purpose = null; },
    value => { value.leaves[0].rocket = true; },
    value => { value.months = [month(0, 9), month(5, 19)]; },
    value => { value.months = [month(0, 9, { converted: { totalRaw: null, reason: null } })]; },
    value => { value.months = [{ ...month(0, 9), other: undefined as never }]; },
  ];
  for (const change of bad) {
    const value = reply();
    change(value);
    assert.throws(() => validateFactorySpendingReply(value));
  }
});

test('new and converted input group by type and design, converted input under its target design', () => {
  const items = factoryItems(reply(), { from: null, hour: 171, end: 171 });
  assert.deepEqual(items.filter(item => item.present).map(item => [item.path.map(part => part.label), item.exact]), [
    [['New', 'Fighter', 'Kestrel II'], 3000000n],
    [['Converted', 'Fighter', 'Kestrel II'], 700000n],
  ]);
  const tree = groupTree(items, { id: 'all', label: 'All military factory IC' });
  assert.equal(tree.exact, 3700000n);
  // A range subtracts its baseline within one observation.
  const range = factoryItems(reply(), { from: 148, hour: 171, end: 171 }).filter(item => item.present);
  assert.deepEqual(range.map(item => [item.path[0].id, item.exact]), [['converted', 700000n]]);
});

test('input whose purpose was not recorded is its own part, never new, and keeps both purposes uncertain', () => {
  const value = reply();
  // 5 IC whose purpose was not recorded beside 30 IC new and 7 IC converted.
  value.leaves.push(leaf({ purpose: 'unknown', runs: [known(100, 171, '500000')], unproven: [known(100, 171, '500000')] }));
  const items = factoryItems(value, { from: null, hour: 171, end: 171 });
  const tree = groupTree(items, { id: 'all', label: 'All' });
  const node = (key: string) => tree.children.find(child => child.key === key)!;
  assert.equal(tree.valueUnknown, false, 'the whole stays known');
  assert.equal(tree.exact, 4_200_000n);
  // Never an exact 5 IC design under New: the uncertain nodes' designs
  // carry the exact amounts.
  assert.equal(purposeSum(items, 'new'), 3_000_000n);
  assert.equal(purposeSum(items, 'purpose-unrecorded'), 500_000n);
  for (const key of ['new', 'converted', 'purpose-unrecorded']) assert.equal(node(key).valueUnknown, true, key);
  const part = items.find(item => item.path[0].id === 'purpose-unrecorded')!;
  assert.deepEqual(part.localUnknownLevels, [true, true, true]);
  assert.match(items.find(item => item.path[0].id === 'new')!.note ?? '', /converted/);
});

test('an unresolved design of a purpose military production never has leaves the whole uncertain, whichever rings show', () => {
  const value = reply();
  // 1 IC new planes beside 2 IC of refits of a design content cannot type.
  value.leaves = [leaf({ runs: [known(100, 171, '100000')] }),
    leaf({ variant: '99', typeKey: ':mod_unknown', name: null, domain: null, names: [], purpose: 'refit', runs: [known(100, 171, '200000')] })];
  const items = factoryItems(value, { from: null, hour: 171, end: 171 });
  for (const shown of [[true, true, true], [false, false, true], [false, true, false]]) {
    assert.equal(groupTree(items, { id: 'all', label: 'All' }, shown).valueUnknown, true, JSON.stringify(shown));
  }
});

test('air input of a purpose military production never has stays in the whole as Other purposes', () => {
  const value = reply();
  value.leaves.push(leaf({ purpose: 'refit', runs: [known(100, 171, '200000')] }));
  const tree = groupTree(factoryItems(value, { from: null, hour: 171, end: 171 }), { id: 'all', label: 'All' });
  assert.equal(tree.exact, 3_900_000n);
  assert.equal(tree.children.find(child => child.key === 'other-purpose')?.exact, 200_000n);
});

test('unread equipment spending and unresolved designs are unknown, never zero', () => {
  const unread = reply();
  unread.leaves = [];
  unread.equipmentTotal = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'notRecorded' }];
  const parts = factoryItems(unread, { from: null, hour: 171, end: 171 }).filter(item => item.path[1]?.id === 'not-known');
  assert.deepEqual(parts.map(item => item.path[0].id), ['new', 'converted']);
  assert.ok(parts.every(item => item.present && item.unknownValue));

  const unresolved = reply();
  unresolved.leaves = [leaf({ variant: '99', typeKey: ':mod_unknown', name: null, domain: null, names: [] })];
  const items = factoryItems(unresolved, { from: null, hour: 171, end: 171 }).filter(item => item.present);
  assert.deepEqual(items.map(item => item.path.map(part => part.id)), [['new', 'unresolved']]);
  assert.deepEqual(items[0].unknownLevels, [true, true, false]);
});

test('months become new and converted columns, and an unknown or unresolved part leaves the total unknown', () => {
  const months = [month(0, 9), month(10, 19, { converted: amount(null) }), month(20, 29, { unresolved: true }),
    month(30, 39, { purposeUnproven: true, unrecorded: amount('50000'), other: amount('3000') })];
  assert.deepEqual(factoryColumns(months, 5), [
    { firstHour: 0, lastHour: 9, values: [1, -0.2, 0, 0], uncertain: [false, false, false, false] },
    { firstHour: 10, lastHour: 19, values: [1, null, 0, 0], uncertain: [false, false, false, false] },
    { firstHour: 20, lastHour: 29, values: [1, -0.2, 0, 0], uncertain: [true, true, false, false] },
    { firstHour: 30, lastHour: 39, values: [1, -0.2, 0.5, 0.03], uncertain: [true, true, true, false] },
  ]);
  assert.deepEqual(factoryMonthTotal(months[0]), { exact: 80_000n, reason: null });
  assert.deepEqual(factoryMonthTotal(months[1]), { exact: null, reason: 'recordingGap' });
  assert.deepEqual(factoryMonthTotal(months[2]), { exact: null, reason: 'unresolved' });
  // Unrecorded and other-purpose input stay in the month's whole.
  assert.deepEqual(factoryMonthTotal(months[3]), { exact: 133_000n, reason: null });
  assert.equal(factoryMonthNote(months[0]), undefined);
  assert.match(factoryMonthNote(months[3]) ?? '', /converted/);
  assert.ok(factoryMonthNote(months[2]));
});

test('equipment and view are settings, and each but the default is its own card', () => {
  assert.deepEqual(PURPOSES.map(purpose => purpose.label), ['New', 'Converted']);
  assert.equal(OPTIONS.find(field => field.key === 'equipment')?.default, 'air');
  assert.equal(OPTIONS.find(field => field.key === 'view')?.default, 'circle');
  assert.deepEqual(definition.variants.map(variant => variant.initial), [
    { title: 'Air factory spending by month', options: { view: 'months' } },
    { title: 'Land factory spending', options: { equipment: 'land' } },
    { title: 'Land factory spending by month', options: { equipment: 'land', view: 'months' } },
  ]);
  assert.deepEqual(definition.timeModes, ['campaign', 'follow', 'pin', 'range']);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import type { ConstructionSpendingReply, SpendingLeaf } from '../../generated/dashboard';
import { foldTree, groupTree } from '../../charts/sunburst/layout.ts';
import { REPAIR_NOTE, footnote, formatSpent, spendingItems, totalAt, validateSpendingReply } from './spendingData.ts';

const leaf = (extra: Partial<SpendingLeaf>): SpendingLeaf => ({
  repair: false, kind: 'building', building: 0, state: 5, toState: null, spent: [], unknown: [], projects: [{ project: 'p1', knownAt: null, firstRead: 100, until: null, unread: false, resumes: null }], ...extra,
});

/** A reply as a modded campaign might give it: a building type only a mod declares
 * (named from the mod's localisation), one the content does not name, and a railway. */
const reply = (): ConstructionSpendingReply => ({
  branchId: '1', tag: 'D01', costScale: 100_000, reads: [100, 124, 148], railwayReads: [100, 124, 148], repairIcon: null,
  types: [
    { id: 20013, token: 'industrial_complex', name: 'Civilian Factory', icon: 'data:image/png;base64,AA' },
    { id: 90001, token: 'mod_fusion_plant', name: 'Fusion Plant', icon: null },
    { id: 90002, token: 'mod_unnamed', name: null, icon: null },
    { id: 12203, token: 'rail_way', name: 'Railway', icon: null },
  ],
  states: [{ id: 5, name: 'Ruhr' }, { id: 6, name: null }, { id: 40, name: 'Berlin' }, { id: 41, name: 'Hamburg' }],
  beforeFirstRead: 2,
  warnings: [],
  leaves: [
    leaf({ spent: [{ knownAt: 124, totalRaw: '7000000' }, { knownAt: 148, totalRaw: '8000000' }] }),
    leaf({ building: 1, state: 6, projects: [{ project: 'p2', knownAt: null, firstRead: 124, until: null, unread: false, resumes: null }], spent: [{ knownAt: 148, totalRaw: '250000' }], unknown: [{ knownAt: 124, reason: 'builtBeforeRead', project: '9' }] }),
    leaf({ building: 2, state: 5, spent: [{ knownAt: 124, totalRaw: '1' }] }),
    leaf({ repair: true, building: 0, state: 5, projects: [{ project: 'p3', knownAt: null, firstRead: 100, until: null, unread: false, resumes: null }, { project: 'p4', knownAt: null, firstRead: 124, until: null, unread: false, resumes: null }] }),
    leaf({ kind: 'railway', building: 3, state: 40, toState: 41, spent: [{ knownAt: 124, totalRaw: '2000000' }], unknown: [{ knownAt: 148, reason: 'leftUnfinished', project: '10' }] }),
  ],
});

test('a well-formed reply passes and a malformed one is refused', () => {
  const good = reply();
  assert.equal(validateSpendingReply(good), good);
  const repairWithAmount = reply();
  repairWithAmount.leaves[3].spent = [{ knownAt: 124, totalRaw: '5' }];
  assert.throws(() => validateSpendingReply(repairWithAmount));
  const outOfOrder = reply();
  outOfOrder.leaves[0].spent.reverse();
  assert.throws(() => validateSpendingReply(outOfOrder));
  const badSpan = reply();
  badSpan.leaves[0].projects = [{ project: 'p5', knownAt: null, firstRead: 124, until: 100, unread: false, resumes: null }];
  assert.throws(() => validateSpendingReply(badSpan));
  const unknownReason = reply();
  (unknownReason.leaves[1].unknown[0] as { reason: string }).reason = 'guess';
  assert.throws(() => validateSpendingReply(unknownReason));
  const badType = reply();
  badType.leaves[0].building = 9;
  assert.throws(() => validateSpendingReply(badType));
});

test('amounts are the exact running totals established by the hour shown', () => {
  const data = reply();
  assert.equal(totalAt(data.leaves[0], 99), 0n);
  assert.equal(totalAt(data.leaves[0], 124), 7_000_000n);
  assert.equal(totalAt(data.leaves[0], 147), 7_000_000n);
  assert.equal(totalAt(data.leaves[0], 500), 8_000_000n);
  assert.equal(formatSpent(8_000_000, null, 100_000), '80');
  assert.equal(formatSpent(250_000, null, 100_000), '2.5');
  assert.equal(formatSpent(1, null, 100_000), '0');
  // Past 2^53 the exact integer is written, not the number (which would read .42).
  assert.equal(formatSpent(Number(9007199254741499n), 9007199254741499n, 100_000), '90,071,992,547.41');
});

test('an inferred repair is listed only from the read that establishes its earlier interval', () => {
  const data = reply();
  data.leaves = [leaf({ repair: true, projects: [{ project: 'r', firstRead: 100, knownAt: 124, until: 124, unread: false, resumes: null }] })];
  assert.equal(validateSpendingReply(data), data);
  for (const hour of [100, 123]) {
    const [repair] = spendingItems(data, { from: null, hour, end: 148 });
    assert.equal(repair.present, false);
    assert.deepEqual(repair.counted, []);
    assert.equal(groupTree([repair], { id: 'all', label: 'All' }).present, false);
  }
  const atRead = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.equal(atRead.present, true);
  assert.deepEqual(atRead.counted, ['r']);
  for (const from of [100, 110]) {
    const [repair] = spendingItems(data, { from, hour: 130, end: 130 });
    assert.equal(repair.present, true);
    assert.deepEqual(repair.counted, ['r']);
  }
  assert.equal(spendingItems(data, { from: 124, hour: 130, end: 130 })[0].present, false);
  assert.equal(spendingItems(data, { from: 100, hour: 123, end: 123 })[0].present, false);
  for (const knownAt of [99, 124.5, NaN, '124']) {
    const bad = structuredClone(data);
    (bad.leaves[0].projects[0] as { knownAt: unknown }).knownAt = knownAt;
    assert.throws(() => validateSpendingReply(bad), /malformed/);
  }
});

test('items name each level from content, keep a mod type, and say why repairs have no amount', () => {
  const items = spendingItems(reply(), { from: null, hour: 124, end: 148 });
  // The mod's type is first read at 124, so it is there; nothing is invented before.
  assert.equal(items.length, 5);
  const [civilian, fusion, unnamed, repair, railway] = items;
  assert.deepEqual(civilian.path.map(key => key.label), ['New buildings', 'Civilian Factory', 'Ruhr']);
  assert.equal(civilian.value, 7_000_000);
  assert.equal(civilian.final, 8_000_000);
  assert.deepEqual(fusion.path.map(key => key.label), ['New buildings', 'Fusion Plant', 'State 6']);
  assert.equal(fusion.path[1].id, 'type:mod_fusion_plant');
  assert.equal(fusion.value, 0);
  assert.deepEqual(fusion.unknown, { builtBeforeRead: 1 });
  // A type the content does not name shows its script name, never a blank.
  assert.equal(unnamed.path[1].label, 'mod_unnamed');
  assert.equal(repair.value, null);
  assert.equal(repair.note, REPAIR_NOTE);
  assert.deepEqual(repair.counted, ['p3', 'p4']);
  assert.deepEqual(railway.path.map(key => key.label), ['New buildings', 'Railway', 'Berlin to Hamburg']);
  // An unknown stretch shows only from the read that establishes it.
  assert.deepEqual(railway.unknown, {});
  assert.deepEqual(spendingItems(reply(), { from: null, hour: 148, end: 148 })[4].unknown, { leftUnfinished: 1 });
  // Before a leaf's first project is read, it is not present: only its whole amount counts.
  const early = spendingItems(reply(), { from: null, hour: 100, end: 148 });
  assert.equal(early.length, 5);
  assert.equal(early[1].present, false);
  assert.equal(early[1].final, 250_000);
});

test('a date range counts only what its reads establish inside it', () => {
  const items = spendingItems(reply(), { from: 125, hour: 148, end: 148 });
  assert.equal(items[0].value, 1_000_000);
  assert.equal(items[1].value, 250_000);
  assert.deepEqual(items[1].unknown, {});
  assert.equal(items[4].value, 0);
});

test('the tree groups by level, and a level switched off joins its parts', () => {
  const items = spendingItems(reply(), { from: null, hour: 148, end: 148 });
  const tree = foldTree(groupTree(items, { id: 'all', label: 'All' }));
  assert.deepEqual(tree.children.map(child => [child.label, child.value, child.unmeasured]), [['New buildings', 10_250_001, false], ['Repairs', 0, true]]);
  const byType = groupTree(items, { id: 'all', label: 'All' }, [false, true, false]);
  const civilian = byType.children.find(child => child.label === 'Civilian Factory')!;
  // New and repair civilian factories join; the repair part is still said to be unmeasured.
  assert.equal(civilian.value, 8_000_000);
  assert.deepEqual(civilian.notes, [REPAIR_NOTE]);
  assert.equal(civilian.count, 3);
});

test('with building types off, a railway within one state joins that state', () => {
  const data = reply();
  data.leaves = [
    leaf({ spent: [{ knownAt: 124, totalRaw: '100' }] }),
    leaf({ kind: 'railway', building: 3, state: 5, toState: 5, spent: [{ knownAt: 124, totalRaw: '50' }] }),
  ];
  const items = spendingItems(data, { from: null, hour: 148, end: 148 });
  const byState = groupTree(items, { id: 'all', label: 'All' }, [false, false, true]);
  assert.deepEqual(byState.children.map(child => [child.label, child.value]), [['Ruhr', 150]]);
});

test('the footnote counts projects whose earlier progress is left out', () => {
  assert.match(footnote(reply())!, /^2 projects were already under way/);
  assert.equal(footnote({ ...reply(), beforeFirstRead: 0 }), undefined);
});

test('Fill sweeps toward the whole of every leaf, including one that appears later', () => {
  const data = reply();
  data.leaves = [
    leaf({ spent: [{ knownAt: 124, totalRaw: '10000000' }] }),
    leaf({ building: 1, state: 6, projects: [{ project: 'p6', knownAt: null, firstRead: 148, until: null, unread: false, resumes: null }], spent: [{ knownAt: 148, totalRaw: '90000000' }] }),
  ];
  const tree = groupTree(spendingItems(data, { from: null, hour: 124, end: 148 }), { id: 'all', label: 'All' });
  assert.equal(tree.value, 10_000_000);
  assert.equal(tree.final, 100_000_000);
});

test('a leaf with only an unknown stretch is kept, with its reason', () => {
  const data = reply();
  data.leaves = [leaf({ unknown: [{ knownAt: 124, reason: 'notRecorded', project: '1' }] })];
  const [only] = spendingItems(data, { from: null, hour: 124, end: 148 });
  assert.equal(only.present, true);
  assert.equal(only.value, 0);
  assert.deepEqual(only.unknown, { notRecorded: 1 });
});

test('a leaf total past i64, added in i128, is valid', () => {
  const data = reply();
  data.leaves[0].spent = [{ knownAt: 124, totalRaw: '108000000000000000000' }];
  assert.equal(validateSpendingReply(data), data);
});

test('exact totals past 2^53 stay exact', () => {
  const data = reply();
  data.leaves = [leaf({ spent: [{ knownAt: 124, totalRaw: '9007199254741499' }] })];
  const [only] = spendingItems(data, { from: null, hour: 124, end: 124 });
  assert.equal(only.exact, 9007199254741499n);
});

test('a date range counts only the projects in the queue during it', () => {
  const data = reply();
  // A repair from 100 until the read at 124 showed it gone; the range is 148..172.
  data.leaves = [leaf({ repair: true, projects: [{ project: 'p7', knownAt: null, firstRead: 100, until: 124, unread: false, resumes: null }] }), leaf({ projects: [{ project: 'p8', knownAt: null, firstRead: 100, until: null, unread: false, resumes: null }] })];
  const [repair, open] = spendingItems(data, { from: 148, hour: 172, end: 172 });
  assert.equal(repair.present, false);
  assert.deepEqual(repair.counted, []);
  assert.equal(open.present, true);
  assert.deepEqual(open.counted, ['p8']);
  // Over the whole campaign the repair is there.
  assert.equal(spendingItems(data, { from: null, hour: 172, end: 172 })[0].present, true);
});

test('a leaf with an unknown stretch established at the start of a range is present', () => {
  const data = reply();
  // Its only project was in the queue 100..124; the read at 124 shows it finished
  // after its last read, which is not counted.
  data.leaves = [leaf({ projects: [{ project: 'a', knownAt: null, firstRead: 100, until: 124, unread: false, resumes: null }], unknown: [{ knownAt: 124, reason: 'finishedAfterRead', project: 'a' }] })];
  const [only] = spendingItems(data, { from: 124, hour: 148, end: 148 });
  assert.equal(only.present, true);
  assert.deepEqual(only.unknown, { finishedAfterRead: 1 });
  assert.equal(spendingItems(data, { from: 125, hour: 148, end: 148 })[0].present, false);
});

test('a project with two intervals of a kind counts once, and only in them', () => {
  const data = reply();
  data.leaves = [leaf({ repair: true, projects: [{ project: 'r', knownAt: null, firstRead: 100, until: 124, unread: false, resumes: null }, { project: 'r', knownAt: null, firstRead: 148, until: null, unread: false, resumes: null }] })];
  assert.equal(spendingItems(data, { from: 125, hour: 147, end: 147 })[0].present, false);
  assert.deepEqual(spendingItems(data, { from: null, hour: 172, end: 172 })[0].counted, ['r']);
});

test('a range across a break in a project\'s reads says so, never zero', () => {
  const data = reply();
  // Read through 124, not recorded 125..219, read again from 220.
  data.leaves = [leaf({ projects: [{ project: 'a', knownAt: null, firstRead: 100, until: 125, unread: true, resumes: 220 }, { project: 'a', knownAt: null, firstRead: 220, until: null, unread: false, resumes: null }] })];
  const [inside] = spendingItems(data, { from: 148, hour: 172, end: 172 });
  assert.equal(inside.present, true);
  assert.deepEqual(inside.unknown, { notRecorded: 1 });
  assert.deepEqual(inside.counted, ['a']);
  const [after] = spendingItems(data, { from: 221, hour: 230, end: 230 });
  assert.deepEqual(after.unknown, {});
});

test('reads that never resume leave the rest of the branch not recorded', () => {
  const data = reply();
  data.leaves = [leaf({ projects: [{ project: 'a', knownAt: null, firstRead: 100, until: 124, unread: true, resumes: null }] })];
  const [late] = spendingItems(data, { from: 130, hour: 140, end: 140 });
  assert.equal(late.present, true);
  assert.deepEqual(late.unknown, { notRecorded: 1 });
});

test('a break and the uncounted stretch that closes it count once', () => {
  const data = reply();
  data.leaves = [leaf({
    projects: [{ project: 'a', knownAt: null, firstRead: 100, until: 125, unread: true, resumes: 220 }, { project: 'a', knownAt: null, firstRead: 220, until: null, unread: false, resumes: null }],
    unknown: [{ knownAt: 220, reason: 'notRecorded', project: 'a' }],
  })];
  assert.deepEqual(spendingItems(data, { from: 100, hour: 230, end: 230 })[0].unknown, { notRecorded: 1 });
  // A range ending inside the break: the closing read is past it, so the break counts.
  assert.deepEqual(spendingItems(data, { from: 148, hour: 172, end: 172 })[0].unknown, { notRecorded: 1 });
});

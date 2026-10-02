import assert from 'node:assert/strict';
import test from 'node:test';
import type { ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun } from '../../generated/dashboard';
import { groupTree, amountText, arcTargets, sliceTooltip } from '../../charts/sunburst/layout.ts';
import { UNIT, formatDeliveredIc, productionSpendingItems, spendingAt, spendingFinals, spendingFootnote, spendingTotal, validateProductionSpendingReply } from './spendingData.ts';
import { definition, OPTIONS } from './definition.ts';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions.ts';
import { resolveOptions } from '../../dashboard/options.ts';

const known = (firstHour: number, lastHour: number, totalRaw: string): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw, reason: null });
const leaf = (extra: Partial<ProductionSpendingLeaf> = {}): ProductionSpendingLeaf => ({
  variant: 'v/1', typeKey: 'mod_mechanised', name: 'Clockwork walkers', domain: 'land',
  names: [{ firstHour: 100, name: 'Early walker', known: true, design: null }, { firstHour: 148, name: 'Revised walker', known: true, design: null }],
  runs: [known(100, 123, '0'), known(124, 147, '1234567'), known(148, 171, '2000001')], ...extra,
});

const reply = (): ProductionSpendingReply => ({
  branchId: '1', tag: 'opaque/tag', decimals: 5, reads: [100, 124, 148], observations: [100], warnings: [],
  leaves: [
    leaf(),
    leaf({ variant: 'v/2', typeKey: 'mod_ornithopter', name: 'Ornithopters', domain: 'air', names: [],
      runs: [known(100, 123, '0'), known(124, 147, '1500000'), known(148, 171, '4000000')] }),
    leaf({ variant: 'v/3', typeKey: 'mod_bathysphere', name: null, domain: 'naval', names: [],
      runs: [known(100, 147, '0'), known(148, 171, '5000000')] }),
  ],
  total: [known(100, 123, '0'), known(124, 147, '2734567'), known(148, 171, '11000001')],
});

const ROOT = { id: 'all', label: 'All production' };
const text = { format: formatDeliveredIc, unit: UNIT, valueLabel: 'Delivered IC at read cost' };

test('a well-formed production spending reply passes; malformed shape is refused', () => {
  const good = reply();
  assert.equal(validateProductionSpendingReply(good), good);
  const changes: ((bad: ProductionSpendingReply) => void)[] = [
    bad => { bad.decimals = 10; },
    bad => { bad.leaves[0].runs[1].totalRaw = '-1'; },
    bad => { bad.leaves[0].runs[1].totalRaw = '1.5'; },
    bad => { bad.leaves[0].runs[1].totalRaw = '1'.repeat(40); },
    bad => { bad.leaves[0].runs[1].totalRaw = null; },
    bad => { bad.leaves[0].runs.reverse(); },
    bad => { bad.leaves[0].runs[1].firstHour = 123; },
    bad => { bad.reads.reverse(); },
    bad => { bad.observations.push(100); },
    bad => { bad.leaves[0].names.reverse(); },
    bad => { (bad.leaves[0].names[0] as { design?: string | null }).design = undefined; },
    bad => { bad.leaves.push(bad.leaves[0]); },
  ];
  for (const change of changes) {
    const bad = reply();
    change(bad);
    assert.throws(() => validateProductionSpendingReply(bad), /malformed/);
  }
  for (const value of [null, {}, [], { ...reply(), leaves: [null] }, { ...reply(), total: [null] }]) {
    assert.throws(() => validateProductionSpendingReply(value), /malformed/);
  }
});

test('the amount at an hour is the exact supplying read, without interpolation or carry across a gap', () => {
  const data = reply();
  assert.deepEqual(spendingAt(data.total, 99), { exact: null, reason: 'outsideRead' });
  assert.deepEqual(spendingAt(data.total, 124), { exact: 2734567n, reason: null });
  assert.deepEqual(spendingAt(data.total, 147.9), { exact: 2734567n, reason: null });
  assert.deepEqual(spendingAt(data.total, 148), { exact: 11000001n, reason: null });
  assert.deepEqual(spendingAt(data.total, 172), { exact: null, reason: 'outsideRead' });
  assert.equal(formatDeliveredIc(1234567, 1234567n), '12.34567');
  assert.equal(formatDeliveredIc(1, 1n), '0.00001');
  assert.equal(formatDeliveredIc(100000, 100000n), '1');
  assert.equal(formatDeliveredIc(Number(9007199254741499n), 9007199254741499n), '90,071,992,547.41499');
});

test('Live looks up the playhead and Live off shows the whole branch at its final hour', () => {
  const data = reply();
  const live = groupTree(productionSpendingItems(data, { from: null, hour: 124, end: 171 }), ROOT);
  assert.equal(live.exact, 2734567n);
  assert.equal(amountText(live, text), '27.34567');
  assert.equal(live.final, 11000001);
  const campaign = groupTree(productionSpendingItems(data, { from: null, hour: 171, end: 171 }), ROOT);
  assert.equal(campaign.exact, 11000001n);
  assert.equal(amountText(campaign, text), '110.00001');
});

test('mod content supplies the types and the shared engine classifiers supply top-level domains', () => {
  const items = productionSpendingItems(reply(), { from: null, hour: 148, end: 171 });
  assert.deepEqual(items.map(item => item.path.map(key => key.label)), [
    ['Land', 'Clockwork walkers', 'Revised walker'], ['Air', 'Ornithopters', 'v/2'], ['Naval', 'mod_bathysphere', 'v/3'],
  ]);
  const tree = groupTree(items, ROOT);
  assert.equal(tree.children.find(node => node.key === 'land')?.exact, 2000001n);
  assert.equal(tree.children.find(node => node.key === 'air')?.exact, 4000000n);
  assert.equal(tree.children.find(node => node.key === 'naval')?.exact, 5000000n);
});

test('variant names are as recorded at the shown hour, with no name backfilled or held past its unknown boundary', () => {
  const data = reply();
  data.leaves[0].names = [{ firstHour: 124, name: 'Read name', known: true, design: null }, { firstHour: 148, name: null, known: false, design: null }];
  assert.equal(productionSpendingItems(data, { from: null, hour: 100, end: 171 })[0].path[2].label, 'v/1');
  assert.equal(productionSpendingItems(data, { from: null, hour: 147, end: 171 })[0].path[2].label, 'Read name');
  assert.equal(productionSpendingItems(data, { from: null, hour: 148, end: 171 })[0].path[2].label, 'v/1');
});

test('an unresolved type stays in the shared unresolved arm, named by its recorded token', () => {
  const data = reply();
  data.leaves.push(leaf({ variant: 'v/unknown', typeKey: ':mod_missing_token', name: null, domain: null, names: [], runs: [known(100, 171, '250000')] }));
  const item = productionSpendingItems(data, { from: null, hour: 148, end: 171 }).at(-1)!;
  assert.deepEqual(item.path.map(key => key.label), ['Type not resolved', 'mod_missing_token', 'v/unknown']);
  assert.equal(item.path[0].id, 'unresolved');
  assert.equal(item.exact, 250000n);
});

test('unresolved attribution makes resolved arms and types unknown while country and exact-design totals stay exact', () => {
  const data = reply();
  data.leaves = [leaf(), leaf({ variant: 'untyped', typeKey: ':mod_unknown', name: null, domain: null, names: [], runs: [known(100, 171, '250000')] })];
  data.total = [known(100, 123, '250000'), known(124, 147, '1484567'), known(148, 171, '2250001')];
  const window = { from: null, hour: 148, end: 171 };
  const items = productionSpendingItems(data, window);
  const tree = groupTree(items, ROOT, [], spendingTotal(data, window));
  assert.equal(tree.valueUnknown, false);
  assert.equal(tree.exact, 2250001n);
  const land = tree.children.find(node => node.key === 'land')!;
  assert.equal(land.valueUnknown, true);
  assert.equal(land.children[0].valueUnknown, true);
  assert.equal(land.children[0].children[0].valueUnknown, false);
  assert.equal(land.children[0].children[0].exact, 2000001n);
  assert.match(land.notes.join(' '), /unresolved/);
  assert.equal(tree.children.find(node => node.key === 'unresolved')?.valueUnknown, false);
  const designs = groupTree(items, ROOT, [false, false, true], spendingTotal(data, window));
  assert.ok(designs.children.every(node => !node.valueUnknown));
  assert.equal(designs.exact, 2250001n);
});

test('unresolved attribution lists a zero arm as unknown without inventing a future observed design', () => {
  const data = reply();
  data.leaves = [leaf(), leaf({ variant: 'untyped', typeKey: ':mod_unknown', name: null, domain: null, names: [], runs: [known(100, 171, '250000')] })];
  data.total = [known(100, 123, '250000'), known(124, 147, '1484567'), known(148, 171, '2250001')];
  const window = { from: null, hour: 100, end: 171 };
  const items = productionSpendingItems(data, window);
  assert.equal(items[0].present, true);
  assert.deepEqual(items[0].counted, []);
  const tree = groupTree(items, ROOT, [], spendingTotal(data, window));
  assert.equal(tree.exact, 250000n);
  assert.equal(tree.children.find(node => node.key === 'land')?.valueUnknown, true);
  const designs = groupTree(items, ROOT, [false, false, true], spendingTotal(data, window));
  assert.equal(designs.children.find(node => node.key === 'v/1')?.valueUnknown, false);
});

test('future unresolved deliveries make grouped Fill endpoints unknown while exact design Fill remains exact', () => {
  const data = reply();
  data.leaves = [leaf(), leaf({ variant: 'untyped', typeKey: ':mod_unknown', name: null, domain: null, names: [],
    runs: [known(100, 147, '0'), known(148, 171, '250000')] })];
  data.total = [known(100, 123, '0'), known(124, 147, '1234567'), known(148, 171, '2250001')];
  const window = { from: null, hour: 124, end: 171 };
  const items = productionSpendingItems(data, window);
  const tree = groupTree(items, ROOT, [], spendingTotal(data, window));
  assert.equal(tree.valueUnknown, false);
  assert.equal(tree.finalUnknown, false);
  const land = tree.children.find(node => node.key === 'land')!;
  assert.equal(land.valueUnknown, false);
  assert.equal(land.finalUnknown, true);
  assert.equal(land.children[0].finalUnknown, true);
  assert.equal(land.children[0].children[0].finalUnknown, false);
  const designs = groupTree(items, ROOT, [false, false, true], spendingTotal(data, window));
  assert.ok(designs.children.every(node => !node.finalUnknown));
});

test('a gap ends what is known; a whole-campaign value never falls back to the last exact read', () => {
  const data = reply();
  data.total = [known(100, 147, '2734567'), { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'recordingGap' }];
  data.leaves[0].runs = [known(100, 147, '1234567'), { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'recordingGap' }];
  const items = productionSpendingItems(data, { from: null, hour: 171, end: 171 });
  const unknown = items[0];
  assert.equal(unknown.exact, null);
  assert.equal(unknown.unknownValue, true);
  assert.equal(unknown.finalUnknown, true);
  assert.match(unknown.note!, /not recorded/);
  const tree = groupTree(items, ROOT);
  assert.equal(tree.valueUnknown, true);
  assert.equal(amountText(tree, text), 'Not known');
  assert.ok([...arcTargets(tree, tree, 'share').values()].every(target => !target.shown));
  assert.equal(tree.children.find(node => node.key === 'air')?.valueUnknown, false);
  assert.match(spendingFootnote(data, { from: null, hour: 171, end: 171 }, 'fill'), /Fill uses Share/);
  assert.match(spendingFootnote(data, { from: null, hour: 171, end: 171 }), /Total not known: Hours were not recorded/);
  assert.doesNotMatch(spendingFootnote(data, { from: null, hour: 171, end: 171 }), /Fill uses Share/);
});

test('a country-level unknown is represented even with no attributable designs', () => {
  const data = reply();
  data.leaves = [];
  data.total = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'missedPublication' }];
  const tree = groupTree(productionSpendingItems(data, { from: null, hour: 124, end: 171 }), ROOT);
  assert.equal(amountText(tree, text), 'Not known');
  assert.match(sliceTooltip(tree, tree, text)[0].note!, /read was missed/);
});

test('a known zero remains exact zero, including a country with no designs', () => {
  const data = reply();
  data.leaves = [];
  data.total = [known(100, 171, '0')];
  const tree = groupTree(productionSpendingItems(data, { from: null, hour: 124, end: 171 }), ROOT);
  assert.equal(tree.valueUnknown, false);
  assert.equal(tree.exact, 0n);
  assert.equal(amountText(tree, text), '0');
});

test('deleted lines retain their delivered value for every later covered hour', () => {
  const data = reply();
  data.leaves = [leaf({ runs: [known(100, 123, '0'), known(124, 171, '1234567')] })];
  data.total = data.leaves[0].runs;
  assert.equal(productionSpendingItems(data, { from: null, hour: 171, end: 171 })[0].exact, 1234567n);
});

test('range totals subtract exact endpoints, including a range beginning when counting began', () => {
  const data = reply();
  const range = groupTree(productionSpendingItems(data, { from: 148, hour: 171, end: 171 }), ROOT);
  assert.equal(range.exact, 11000001n - 2734567n);
  const entire = groupTree(productionSpendingItems(data, { from: 100, hour: 171, end: 171 }), ROOT);
  assert.equal(entire.exact, 11000001n);
  const unrecordedBaseline = groupTree(productionSpendingItems(data, { from: 99, hour: 171, end: 171 }), ROOT);
  assert.equal(unrecordedBaseline.valueUnknown, true);
});

test('a range crossing a gap or observation restart stays unknown', () => {
  const data = reply();
  data.observations = [100, 148];
  data.total = [known(100, 147, '2734567'), { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'observationRestarted' }];
  const tree = groupTree(productionSpendingItems(data, { from: 124, hour: 171, end: 171 }), ROOT);
  assert.equal(tree.valueUnknown, true);
  assert.match(tree.notes.join(' '), /restarted/);
  assert.equal(spendingAt(data.total, 171).exact, null);
});

test('unknown final amounts use earlier exact values for stable ordering only', () => {
  const data = reply();
  data.leaves[0].runs = [known(100, 147, '1234567'), { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'missingCost' }];
  const finals = spendingFinals(data, null, 171);
  assert.deepEqual(finals[0], { exact: null, geometry: 1234567 });
  const before = productionSpendingItems(data, { from: null, hour: 124, end: 171 }, finals)[0];
  const after = productionSpendingItems(data, { from: null, hour: 171, end: 171 }, finals)[0];
  assert.equal(before.final, after.final);
  assert.equal(before.exact, 1234567n);
  assert.equal(after.exact, null);
  assert.equal(before.finalUnknown, true);
  assert.equal(after.finalUnknown, true);
});

test('a later country-level gap marks the whole endpoint unknown without replacing the current exact amount', () => {
  const data = reply();
  data.total = [known(100, 123, '0'), known(124, 147, '2734567'), { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'recordingGap' }];
  const tree = groupTree(productionSpendingItems(data, { from: null, hour: 124, end: 171 }), ROOT);
  assert.equal(tree.valueUnknown, false);
  assert.equal(tree.finalUnknown, true);
  assert.equal(tree.exact, 2734567n);
});

test('all sunburst settings and levels default to the shared construction behaviour', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.equal(definition.defaultTime, 'campaign');
  assert.equal(options.refits, false);
  assert.deepEqual(levelsShown(options, ['domainLevel', 'typeLevel', 'variantLevel']), [true, true, true]);
  assert.deepEqual(sunburstSettings(options), { growth: 'share', labels: true, legend: true, breadcrumb: true, dimming: true });
  const flat = groupTree(productionSpendingItems(reply(), { from: null, hour: 171, end: 171 }), ROOT, [false, false, false]);
  assert.equal(flat.exact, 11000001n);
  assert.deepEqual(flat.children, []);
});

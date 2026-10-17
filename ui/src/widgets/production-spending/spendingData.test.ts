import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun } from '../../generated/dashboard';
import type { PreparedProductionSpendingReply } from './spendingData';
import { groupTree, amountText, arcTargets, sliceTooltip } from '../../charts/sunburst/layout.ts';
import type { SunburstText } from '../../charts/sunburst/layout';
import { UNIT, formatAppliedIc, preparedWindow, productionSpendingItems, spendingAt, spendingEmptyLabel, spendingFinals, spendingTotal, validateProductionSpendingReply } from './spendingData.ts';
import { definition, OPTIONS } from './definition.ts';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { icDecimals } from '../../dashboard/icDecimals.ts';

const known = (firstHour: number, lastHour: number, totalRaw: string): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw, reason: null });
const leaf = (extra: Partial<ProductionSpendingLeaf> = {}): ProductionSpendingLeaf => ({
  variant: 'v/1', typeKey: 'mod_mechanised', name: 'Clockwork walkers', domain: 'land', rocket: false,
  names: [{ firstHour: 100, name: 'Early walker', known: true, design: null }, { firstHour: 148, name: 'Revised walker', known: true, design: null }],
  runs: [known(100, 123, '0'), known(124, 147, '1234567'), known(148, 171, '2000001')], ...extra,
});

const reply = (): ProductionSpendingReply => ({
  branchId: '1', tag: 'opaque/tag', decimals: 5, reads: [100, 124, 148], observations: [100], warnings: [],
  equipmentReads: [100, 124, 148], rocketReads: [100, 124, 148], equipmentObservations: [100], rocketObservations: [100],
  rocketKey: 'mod_launch_works', rocketName: 'Launch works', rocketTotal: [known(100, 171, '0')],
  production: [{ firstHour: 100, lastHour: 171, present: true, emptyCensusHour: null }],
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
const text: SunburstText = { format: (value, exact) => formatAppliedIc(value, exact, 5, 5), unit: UNIT, valueLabel: 'Equipment-line IC' };

test('a well-formed production spending reply passes; malformed shape is refused', () => {
  const good = reply();
  assert.equal(validateProductionSpendingReply(good), good);
  const changes: ((bad: ProductionSpendingReply) => void)[] = [
    bad => { bad.decimals = 10; },
    bad => { bad.leaves[0].runs[1].totalRaw = '--1'; },
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
  assert.equal(formatAppliedIc(1234567, 1234567n, 5, 5), '12.34567');
  assert.equal(formatAppliedIc(1, 1n, 5, 5), '0.00001');
  assert.equal(formatAppliedIc(100000, 100000n, 5, 5), '1');
  assert.equal(formatAppliedIc(Number(9007199254741499n), 9007199254741499n, 5, 5), '90,071,992,547.41499');
});

test('IC amounts are whole numbers by default, and the settings can ask for every recorded digit', () => {
  const total = 1685606865n;
  const written = (stored: Record<string, unknown>) => formatAppliedIc(Number(total), total, 5, icDecimals(resolveOptions(OPTIONS, stored as never), 5));
  assert.equal(written({}), '16,856');
  assert.equal(written({ icDecimals: 2 }), '16,856.07');
  assert.equal(written({ icDecimals: 5 }), '16,856.06865');
  // A stored value out of range keeps the default rather than inventing digits.
  assert.equal(written({ icDecimals: 9 }), '16,856');
  assert.equal(formatAppliedIc(Number(9007199254741499n), 9007199254741499n, 5, 0), '90,071,992,547');
});

test('complete-empty evidence validation scales with its ordered read inventory', () => {
  const data = reply(), reads = Array.from({ length: 5000 }, (_, index) => 100 + index);
  let accesses = 0;
  data.equipmentReads = new Proxy(reads, { get(target, key, receiver) {
    if (typeof key === 'string' && /^\d+$/.test(key)) accesses++;
    return Reflect.get(target, key, receiver);
  } });
  data.production = reads.map(hour => ({ firstHour: hour, lastHour: hour, present: false, emptyCensusHour: hour }));
  assert.equal(validateProductionSpendingReply(data), data);
  assert.ok(accesses < reads.length * 40, `${accesses} read accesses`);
  data.production[0].emptyCensusHour = 99;
  assert.throws(() => validateProductionSpendingReply(data), /malformed/);
});

test('signed cumulative corrections and negative range changes remain selectable exact rows', () => {
  const data = reply();
  data.leaves = [leaf({ runs: [known(100, 123, '400000'), known(124, 171, '-100000')] })];
  data.total = data.leaves[0].runs;
  assert.equal(validateProductionSpendingReply(data), data);
  assert.deepEqual(spendingAt(data.total, 124), { exact: -100000n, reason: null });
  const window = { from: 124, hour: 148, end: 171 };
  const items = productionSpendingItems(data, window);
  assert.equal(items[0].exact, -500000n);
  assert.equal(items[0].present, true);
  assert.deepEqual(items[0].counted, ['v/1']);
  const tree = groupTree(items, ROOT, [], spendingTotal(data, window));
  assert.equal(tree.exact, -500000n);
  assert.equal(amountText(tree, text), '\u22125');
  assert.ok([...arcTargets(tree, tree, 'share').values()].every(target => !target.shown));
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
    ['Land', 'Clockwork walkers', 'Revised walker'], ['Air', 'Ornithopters', 'v/2'], ['Naval', 'Equipment type name unavailable', 'v/3'],
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
  assert.deepEqual(item.path.map(key => key.label), ['Type not resolved', 'Equipment type name unavailable', 'v/unknown']);
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
  assert.match(land.notes.join(' '), /Equipment type unknown/);
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
});

test('a country-level unknown is represented even with no attributable designs', () => {
  const data = reply();
  data.leaves = [];
  data.total = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'missedPublication' }];
  const tree = groupTree(productionSpendingItems(data, { from: null, hour: 124, end: 171 }), ROOT);
  assert.equal(amountText(tree, text), 'Not known');
  assert.match(sliceTooltip(tree, tree, text)[0].note!, /Spending unavailable at this time/);
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

test('an empty publication range needs covered exact endpoints, not a line observation', () => {
  const data = reply();
  data.observations = [];
  data.leaves = [];
  data.total = [known(100, 171, '0')];
  const window = { from: 124, hour: 148, end: 171 };
  const tree = groupTree(productionSpendingItems(data, window), ROOT, [], spendingTotal(data, window));
  assert.equal(tree.valueUnknown, false);
  assert.equal(tree.finalUnknown, false);
  assert.equal(tree.exact, 0n);
  assert.equal(amountText(tree, text), '0');
  assert.equal(spendingTotal(data, { ...window, from: 100 }).unknownValue, true);
  data.total = [known(100, 123, '0'), known(148, 171, '0')];
  assert.equal(spendingTotal(data, window).unknownValue, true);
  data.total = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'notRecorded' }];
  assert.equal(spendingTotal(data, window).unknownValue, true);
  assert.match(spendingTotal(data, window).note!, /not recorded/);
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
  assert.match(tree.notes.join(' '), /Campaign total unavailable/);
  assert.equal(spendingAt(data.total, 171).exact, null);
});

test('unknown final amounts use earlier exact values for stable ordering only', () => {
  const data = reply();
  data.leaves[0].runs = [known(100, 147, '1234567'), { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'spendingUnavailable' }];
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
  assert.equal(OPTIONS.find(option => option.key === 'domainLevel')?.label, 'Production source');
  assert.deepEqual(levelsShown(options, ['domainLevel', 'typeLevel', 'variantLevel']), [true, true, true]);
  assert.deepEqual(sunburstSettings(options), { growth: 'share', labels: true, legend: true, breadcrumb: true, dimming: true });
  const flat = groupTree(productionSpendingItems(reply(), { from: null, hour: 171, end: 171 }), ROOT, [false, false, false]);
  assert.equal(flat.exact, 11000001n);
  assert.deepEqual(flat.children, []);
});

test('Rocket and equipment leaves with the same design stay distinct when rings are hidden', () => {
  const data = reply();
  data.leaves = [leaf({ runs: [known(100, 171, '200000')] }),
    leaf({ rocket: true, runs: [known(100, 171, '-300003')] })];
  data.rocketTotal = data.leaves[1].runs;
  data.total = [known(100, 171, '-100003')];
  assert.equal(validateProductionSpendingReply(data), data);
  const items = productionSpendingItems(data, { from: null, hour: 148, end: 171 });
  assert.equal(items[1].path[0].label, 'Launch works');
  const tree = groupTree(items, ROOT, [false, false, true], spendingTotal(data, { from: null, hour: 148, end: 171 }));
  assert.equal(tree.children.length, 2);
  assert.deepEqual(tree.children.map(node => node.exact), [200000n, -300003n]);
  assert.equal(tree.exact, -100003n);
  assert.ok([...arcTargets(tree, tree, 'share').values()].filter(target => target.shown).length > 0);
});

test('missing Rocket coverage keeps equipment slices exact while the whole remains unknown', () => {
  const data = reply();
  data.rocketReads = []; data.rocketObservations = []; data.rocketTotal = [];
  data.production[0] = { firstHour: 100, lastHour: 171, present: false, emptyCensusHour: 100 };
  data.total = [known(100, 171, '1')];
  assert.doesNotMatch(spendingEmptyLabel(data, 124), /No military production/);
  assert.doesNotMatch(spendingEmptyLabel(data, 99), /No military production/);
  data.total = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'rocketTableUnavailable' }];
  const window = { from: null, hour: 148, end: 171 };
  const tree = groupTree(productionSpendingItems(data, window), ROOT, [], spendingTotal(data, window));
  assert.equal(tree.exact, null);
  assert.equal(tree.children.find(node => node.key === 'land')?.exact, 2000001n);
});

test('unresolved Rocket type does not make unrelated equipment arms unknown', () => {
  const data = reply();
  data.leaves.push(leaf({ rocket: true, variant: 'unknown', domain: null, typeKey: ':mod_rocket', name: null,
    names: [], runs: [known(100, 171, '1')] }));
  const tree = groupTree(productionSpendingItems(data, { from: null, hour: 148, end: 171 }), ROOT);
  assert.equal(tree.children.find(node => node.key === 'land')?.valueUnknown, false);
  assert.equal(tree.children.find(node => node.key === 'rocket')?.exact, 1n);
});

test('mixed resolved and unresolved Rocket types keep source and design amounts independently exact', () => {
  const data = reply();
  data.leaves = [
    leaf({ runs: [known(100, 171, '200000')] }),
    leaf({ rocket: true, variant: 'known_rocket', domain: 'air', typeKey: 'mod_missile', name: 'Mod missiles', names: [],
      runs: [known(100, 123, '5000'), known(124, 147, '10000'), known(148, 171, '25000')] }),
    leaf({ rocket: true, variant: 'untyped_rocket', domain: null, typeKey: ':unknown_rocket', name: null, names: [],
      runs: [known(100, 123, '0'), known(124, 147, '3000'), known(148, 171, '5000')] }),
  ];
  data.rocketTotal = [known(100, 123, '5000'), known(124, 147, '13000'), known(148, 171, '30000')];
  data.total = [known(100, 123, '205000'), known(124, 147, '213000'), known(148, 171, '230000')];
  for (const hour of [100, 124]) {
    const window = { from: null, hour, end: 171 };
    const items = productionSpendingItems(data, window);
    for (const shown of [[true, true, true], [true, false, true]]) {
      const tree = groupTree(items, ROOT, shown, spendingTotal(data, window));
      const rocket = tree.children.find(node => node.key === 'rocket')!;
      assert.equal(rocket.valueUnknown, false);
      assert.equal(rocket.exact, hour === 100 ? 5000n : 13000n);
      assert.equal(rocket.finalUnknown, false);
      assert.equal(rocket.final, 30000);
      const design = shown[1] ? rocket.children.find(node => node.key === 'mod_missile')!.children[0]
        : rocket.children.find(node => node.key === 'rocket:known_rocket')!;
      assert.equal(design.valueUnknown, false);
      assert.equal(design.exact, hour === 100 ? 5000n : 10000n);
      assert.equal(design.finalUnknown, false);
      assert.equal(design.final, 25000);
      if (shown[1]) {
        const type = rocket.children.find(node => node.key === 'mod_missile')!;
        assert.equal(type.valueUnknown, hour === 124);
        assert.equal(type.finalUnknown, true);
      }
      const land = tree.children.find(node => node.key === 'land')!;
      assert.equal(land.valueUnknown, false);
      assert.equal(land.exact, 200000n);
      assert.equal(land.finalUnknown, false);
      assert.equal(land.final, 200000);
    }
  }
});

test('empty-state wording requires dated complete-empty census evidence and exact zero', () => {
  const data = reply();
  data.total = [known(100, 171, '0')];
  data.production = [{ firstHour: 100, lastHour: 171, present: false, emptyCensusHour: 100 }];
  assert.equal(spendingEmptyLabel(data, 124), 'No military production yet.');
  assert.doesNotMatch(spendingEmptyLabel(data, 124, 99), /No military production/);
  data.production[0].emptyCensusHour = null;
  assert.equal(spendingEmptyLabel(data, 124), 'No factory IC spent in this view.');
  data.production[0].present = true;
  assert.equal(spendingEmptyLabel(data, 124), 'No factory IC spent in this view.');
  data.production[0].present = null;
  assert.match(spendingEmptyLabel(data, 124), /No factory IC spending recorded/);
  data.production[0] = { firstHour: 100, lastHour: 171, present: false, emptyCensusHour: 100 };
  data.total = [known(100, 171, '1')];
  assert.doesNotMatch(spendingEmptyLabel(data, 124), /No military production/);
  assert.doesNotMatch(spendingEmptyLabel(data, 99), /No military production/);
  data.total = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'rocketTableUnavailable' }];
  assert.match(spendingEmptyLabel(data, 124), /Rocket spending not recorded/);
  assert.doesNotMatch(spendingEmptyLabel(data, 124), /No military production/);
});

test('prepared spending ranges use native segments and exact extrema rather than sampled endpoints', () => {
  const data: PreparedProductionSpendingReply = reply();
  const wide = 10n ** 25n;
  data.leaves = [leaf({ runs: [known(123, 123, String(wide)), known(148, 148, String(wide + 3n)),
    { firstHour: 171, lastHour: 171, totalRaw: null, reason: 'lostOutput' }] })];
  data.total = [known(123, 123, String(wide)), known(148, 148, String(wide + 3n)),
    { firstHour: 171, lastHour: 171, totalRaw: null, reason: 'lostOutput' }];
  data.continuity = { '148': [true, true], '171': [true, true] };
  data.peaks = { [JSON.stringify(['v/1', false])]: String(wide + 17n) };
  assert.equal(productionSpendingItems(data, { from: 124, hour: 148, end: 171 })[0].exact, 3n);
  assert.deepEqual(spendingFinals(data, 124, 171)[0], { exact: null, geometry: 17 });
  data.continuity['148'] = [false, true];
  assert.equal(productionSpendingItems(data, { from: 124, hour: 148, end: 171 })[0].exact, null);
});

test('a reply answered from a prepared prefix reads at the prefix’s last hour; the later hours are still being prepared', () => {
  const coverage = { schema: 24, firstHour: 100, lastHour: 147, complete: false };
  const preparing = { firstHour: 148, lastHour: 171, totalRaw: null, reason: 'preparing' } as const;
  const prefix = { ...reply(), coverage, pending: [{ firstHour: 148, lastHour: 171 }], total: [known(100, 123, '0'), known(124, 147, '2734567'), preparing],
    leaves: reply().leaves.map(item => ({ ...item, runs: [...item.runs.filter(run => run.lastHour <= 147), preparing] })) };
  validateProductionSpendingReply(prefix);
  assert.throws(() => validateProductionSpendingReply({ ...prefix, pending: [{ firstHour: 171, lastHour: 148 }] }), /malformed/);
  // Campaign view at the branch's end: the amounts spent through the prefix.
  const shown = preparedWindow(prefix, { from: null, hour: 171, end: 171 })!;
  assert.deepEqual(shown, { from: null, hour: 147, end: 147 });
  assert.equal(spendingTotal(prefix, shown).exact, 2734567n);
  assert.equal(spendingAt(prefix.total, 171).reason, 'preparing', 'a later hour is still being prepared, never unread');
  assert.equal(productionSpendingItems(prefix, shown).find(item => item.path[2]?.id === 'v/2')?.exact, 1500000n);
  // A range the prefix has not reached is still being prepared; one inside it is answered.
  assert.equal(preparedWindow(prefix, { from: 148, hour: 171, end: 171 }), null);
  assert.deepEqual(preparedWindow(prefix, { from: 124, hour: 160, end: 171 }), { from: 124, hour: 147, end: 147 });
  // A complete reply answers the requested dates.
  const complete = { ...reply(), coverage: { ...coverage, lastHour: 171, complete: true }, pending: [] };
  assert.deepEqual(preparedWindow(complete, { from: null, hour: 160, end: 171 }), { from: null, hour: 160, end: 171 });
  assert.deepEqual(preparedWindow(reply(), { from: null, hour: 160, end: 171 }), { from: null, hour: 160, end: 171 });
});

test('the actual spending widget shows the census-backed empty state even with its legend disabled', async () => {
  const state = { data: reply(), footnote: undefined as string | undefined, request: undefined as unknown, source: undefined as unknown };
  const host = globalThis as typeof globalThis & { __spendingEmpty?: typeof state };
  host.__spendingEmpty = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'spending-empty-fixture', enforce: 'pre', transform(_source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/dashboard/products.ts')) return 'export const useProduct = (_client, request, _active, _validate, source) => { globalThis.__spendingEmpty.request = request; globalThis.__spendingEmpty.source = source; return { data: globalThis.__spendingEmpty.data }; };';
      if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = () => {};';
      if (path.endsWith('/charts/sunburst/Sunburst.tsx')) return "import { createElement } from 'react'; export default props => { globalThis.__spendingEmpty.footnote = props.footnote; return createElement('div', null, 'Spending chart'); };";
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/production-spending/ProductionSpendingWidget.tsx');
    const props = { widget: { kind: 'production-spending', trackIds: ['track'], countries: { kind: 'tags', tags: ['opaque/tag'] },
      time: { mode: 'campaign' }, options: { legend: false } }, active: true, onDiagnostics() {},
      runtime: { tracks: [{ id: 'track', axisOffset: 0, source: { id: 'source', revision: '1' },
        branch: { id: '1', firstHour: 100, lastHour: 171 } }], client: {}, clock: { getHour: () => 124,
        getSnapshot: () => ({ hoursPerSecond: 24 }) } } } as unknown as WidgetProps;
    state.data.leaves = [];
    state.data.total = [known(100, 171, '0')];
    state.data.production = [{ firstHour: 100, lastHour: 171, present: false, emptyCensusHour: 100 }];
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    assert.match(render(), /No military production yet\./);
    state.data.production[0].emptyCensusHour = null;
    assert.match(render(), /Spending chart/);
    assert.equal(state.footnote, undefined, 'the dense recording footnote is absent from the rendered chart props');
    state.data.production[0] = { firstHour: 100, lastHour: 171, present: true, emptyCensusHour: null };
    assert.match(render(), /Spending chart/, 'zero amounts from active lines do not prove emptiness');
    state.data.total = [{ firstHour: 100, lastHour: 171, totalRaw: null, reason: 'rocketTableUnavailable' }];
    assert.doesNotMatch(render(), /No military production/);
    props.runtime.client.preparedQuery = async () => ({} as never);
    render();
    assert.equal(state.source, props.runtime.tracks[0].source);
    assert.deepEqual((state.request as { params: unknown }).params, { branchId: '1', tag: 'opaque/tag', refits: false, from: null, hour: 171, end: 171 });
  } finally { await vite.close(); delete host.__spendingEmpty; }
});

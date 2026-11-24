import assert from 'node:assert/strict';
import test from 'node:test';
import { arcTargets, listed, sliceColours, sunburstTree, sunburstSummary } from './layout.ts';
import { shareColourReason, shareReading, shareTooltip } from './share.ts';

const root = { id: 'world', label: 'States' };
const rest = { id: 'rest', label: 'Rest of world', colour: '#888888' };
const look = (key: string) => ({ id: key, label: `Country ${key}`, colour: '#797979' });
const text = { format: (value: number, exact: bigint | null) => String(exact ?? value), unit: 'states', valueLabel: 'States', missingLabel: 'Not recorded' };

test('a missing selected entity colour refuses the share instead of supplying a neutral country slice', () => {
  const colours = new Map([['a', '#353535']]);
  assert.equal(shareColourReason(['a'], key => colours.get(key) ?? null), null);
  const reason = shareColourReason(['a', 'b'], key => colours.get(key) ?? null);
  const reading = shareReading([{ key: 'a', value: 5 }, { key: 'b', value: 5 }], 20, 10, look, rest, reason);
  assert.equal(reading.items.length, 0);
  assert.equal(reading.missingLabel, 'Not recorded');
  assert.match(reading.unavailableReason!, /colours.*not recorded/);
  assert.equal(colours.get('a'), '#353535', 'a recorded grey stays exactly as supplied');
});

test('a completed missing flag at the hovered hour replaces the slice artwork', () => {
  const forms = [{ title: 'Old name', icon: 'earlier-flag.png', rows: [{ label: 'States', value: '5' }] }];
  const hovered = shareTooltip(forms, { name: 'Dated name' }, 'Chosen hour');
  assert.equal(hovered[0].title, 'Dated name');
  assert.equal(hovered[0].icon, undefined);
  assert.deepEqual(hovered[0].countryFlag, { name: 'Dated name' });
  assert.equal(hovered[0].subtitle, 'Chosen hour');
  assert.equal(shareTooltip(forms, undefined, 'Chosen hour')[0].icon, 'earlier-flag.png');
  const pending = { name: 'Dated name', flagPending: true };
  const failed = { name: 'Dated name', flagReason: 'Dated flag read failed' };
  assert.equal(shareTooltip(forms, pending, 'Chosen hour')[0].countryFlag, pending);
  assert.equal(shareTooltip(forms, failed, 'Chosen hour')[0].countryFlag, failed);
});

test('a single-ring share keeps all top 30 entity colours and a neutral remainder after them', () => {
  const reading = shareReading(Array.from({ length: 40 }, (_, index) => ({ key: String(index), value: 100 - index })), 10_000, 30, look, rest);
  const tree = sunburstTree(reading.items, root, undefined, { fold: false });
  assert.equal(tree.children.length, 31);
  assert.equal(tree.children.at(-1)!.key, 'rest');
  assert.equal(tree.value, 10_000);
  assert.equal(tree.exact, 10_000n);
  assert.equal(tree.children.some(child => child.other), false);
  const colours = sliceColours(tree, ['#abcdef'], { light: '#ffffff', dark: '#000000' });
  assert.equal(colours.get(tree.children[0].id), '#797979');
  assert.equal(colours.get(tree.children.at(-1)!.id), rest.colour);
  const targets = arcTargets(tree, tree, 'share', 1);
  assert.equal([...targets.values()].filter(target => target.shown).length, 31);
  assert.ok(Math.abs(targets.get(tree.children.at(-1)!.id)!.a1 - Math.PI * 2) < 1e-9);
});

test('one missing candidate or world read replaces the whole ring, rather than a partial share', () => {
  for (const reading of [shareReading([{ key: 'a', value: 5 }], null, 10, look, rest),
    shareReading([{ key: 'a', value: 5 }, { key: 'b', value: null }], 10, 10, look, rest)]) {
    const tree = sunburstTree([{ path: [look('old')], value: 9 }], root, undefined, reading);
    assert.equal(tree.children.length, 0);
    assert.equal(tree.unmeasured, true);
    assert.equal([...arcTargets(tree, tree, 'share', 1).values()].some(target => target.shown), false);
    assert.match(sunburstSummary(tree, text, true), /not recorded/);
  }
});

test('a measured empty world is zero, while negatives and overlapping totals refuse proportions explicitly', () => {
  const empty = shareReading([{ key: 'a', value: 0 }], 0, 10, look, rest);
  const tree = sunburstTree(empty.items, root, undefined, { fold: false });
  assert.equal(tree.unmeasured, false);
  assert.equal(tree.exact, 0n);
  assert.match(sunburstSummary(tree, text), /States: 0 states/);
  assert.equal(tree.children.every(listed), true);
  assert.match(sunburstSummary(tree, text), /Country a: 0 states/);
  assert.equal(sliceColours(tree, ['#abcdef'], { light: '#ffffff', dark: '#000000' }).get(tree.children[0].id), '#797979');
  for (const [values, world] of [[[{ key: 'a', value: -1 }], 10], [[{ key: 'a', value: 11 }], 10]] as const) {
    const reading = shareReading(values, world, 10, look, rest);
    assert.equal(reading.items.length, 0);
    assert.equal(reading.missingLabel, 'Share unavailable');
  }
});

test('a share keeps exact large safe amounts, rejects overflow, and ranks candidates in service tag order and draws ties in main key order', () => {
  const reading = shareReading([{ key: '\u{10000}', value: 5 }, { key: '\ue000', value: 5 }], Number.MAX_SAFE_INTEGER, 1, look, rest);
  assert.equal(reading.items[0].path[0].id, '\ue000');
  assert.equal(reading.items[1].exact, BigInt(Number.MAX_SAFE_INTEGER) - 5n);
  assert.equal(shareReading([{ key: 'a', value: Number.MAX_SAFE_INTEGER }, { key: 'b', value: 1 }], Number.MAX_SAFE_INTEGER, 10, look, rest).items.length, 0);
  const tied = shareReading([{ key: '"', value: 5 }, { key: 'A', value: 5 }, { key: '\u{10000}', value: 5 }, { key: '\ue000', value: 5 }],
    20, 4, key => ({ id: JSON.stringify(['entity', key]), label: key }), rest);
  assert.deepEqual(sunburstTree(tied.items, root, undefined, { fold: false }).children.filter(child => !child.last).map(child => child.label), ['A', '"', '\u{10000}', '\ue000']);
});

test('playback redistributes a full ring while exact labels stay at the supplied reading', () => {
  const angles = [20, 50].map(value => {
    const reading = shareReading([{ key: 'a', value }], 100, 10, look, rest);
    const tree = sunburstTree(reading.items, root, undefined, { fold: false });
    assert.equal(tree.children[0].exact, BigInt(value));
    return arcTargets(tree, tree, 'share', 1).get(tree.children[0].id)!.a1;
  });
  assert.ok(angles[1] > angles[0]);
});

test('5,000 candidates require looks only for the selection and preserve every value', () => {
  const values = Array.from({ length: 5_000 }, (_, index) => ({ key: String(index), value: 5_000 - index }));
  let looks = 0;
  const reading = shareReading(values, 12_502_500, 30, key => { looks++; return look(key); }, rest);
  assert.equal(looks, 30);
  assert.deepEqual(reading.items.map(item => [item.path[0].id, item.value, item.exact]), [
    ...values.slice(0, 30).map(part => [part.key, part.value, BigInt(part.value)]),
    [rest.id, 12_352_935, 12_352_935n],
  ]);
  const all = shareReading(values, 12_502_500, Number.MAX_SAFE_INTEGER, look, rest);
  assert.deepEqual(all.items.map(item => [item.path[0].id, item.value, item.exact]), [
    ...values.map(part => [part.key, part.value, BigInt(part.value)]), [rest.id, 0, 0n],
  ]);
});

test('shared summaries keep main wording unless unavailable-reading notes are requested', () => {
  const tree = sunburstTree([], root, [], { unavailableReason: 'Roster not recorded' });
  assert.equal(sunburstSummary(tree, text), 'States: not recorded');
  assert.equal(sunburstSummary(tree, text, true), 'States: not recorded; Roster not recorded');
});


import assert from 'node:assert/strict';
import test from 'node:test';
import { assertLinear } from '../assertLinear.ts';
import { approachColour, arcTargets, findNode, foldTree, foldView, groupTree, keyRings, keyedSlice, labelFits, pathTo, shareText, sliceAt, listed, sliceColours, sliceTooltip, sunGeometry, sunburstSummary, sweepOf, walk } from './layout.ts';
import type { SunburstItem } from './layout.ts';

const key = (id: string) => ({ id, label: id.toUpperCase() });
const item = (path: string[], value: number | null, extra: Partial<SunburstItem> = {}): SunburstItem => ({ path: path.map(key), value, ...extra });
const ROOT = { id: 'all', label: 'All' };

test('a tree adds its items up and keeps missing measurement apart from zero', () => {
  const root = groupTree([
    item(['new', 'civ', 'ruhr'], 30, { final: 50 }),
    item(['new', 'civ', 'saxony'], 10, { final: 10, unknown: { gap: 1 } }),
    item(['new', 'fort', 'saar'], 0, { final: 5 }),
    item(['repair', 'civ', 'ruhr'], null, { note: 'Not measured' }),
  ], ROOT);
  assert.equal(root.value, 40);
  assert.equal(root.final, 65);
  assert.deepEqual(root.unknown, { gap: 1 });
  assert.deepEqual(root.children.map(child => child.key), ['new', 'repair']);
  const repair = root.children[1];
  assert.equal(repair.unmeasured, true);
  assert.equal(repair.value, 0);
  assert.deepEqual(repair.notes, ['Not measured']);
  assert.equal(root.unmeasured, false);
  // Children keep one order by their whole amount, not by the amount so far.
  const civ = root.children[0].children[0];
  assert.deepEqual(civ.children.map(child => child.key), ['ruhr', 'saxony']);
  assert.deepEqual(root.children[0].children.map(child => child.key), ['civ', 'fort']);
});

test('a level switched off joins its children, adding the same keys together', () => {
  const items = [item(['new', 'civ', 'ruhr'], 30), item(['new', 'fort', 'saar'], 5), item(['repair', 'civ', 'ruhr'], null, { note: 'n' }), item(['old', 'civ', 'ruhr'], 7)];
  const root = groupTree(items, ROOT, [false, true, false]);
  assert.deepEqual(root.children.map(child => [child.key, child.value, child.unmeasured]), [['civ', 37, false], ['fort', 5, false]]);
  // A node with a measured part keeps the note of the part that is not measured.
  assert.deepEqual(root.children[0].notes, ['n']);
  assert.deepEqual(root.children[0].children, []);
  const flat = groupTree(items, ROOT, [false, false, false]);
  assert.equal(flat.children.length, 0);
  assert.equal(flat.value, 42);
});

test('more slices than colours fold their smallest into Other, the same ones all playback long', () => {
  const items = Array.from({ length: 11 }, (_, index) => item([`t${index}`], index === 0 ? 0 : 100 - index, { final: 200 - index }));
  const root = foldTree(groupTree(items, ROOT), 8, 'Other');
  assert.equal(root.children.length, 8);
  const other = root.children.at(-1)!;
  assert.equal(other.other, true);
  // t0 has nothing yet but the most by the end, so it stays out of Other.
  assert.equal(root.children[0].key, 't0');
  assert.deepEqual(other.children.map(child => child.key), ['t7', 't8', 't9', 't10']);
  assert.equal(other.value, 93 + 92 + 91 + 90);
  assert.equal(other.children[0].depth, 2);
  assert.equal(other.children[0].parent, other);
  // A folded slice keeps its id, so a zoom into it survives regrouping.
  assert.equal(findNode(root, other.children[0].id), other.children[0]);
  assert.equal(findNode(root, `${other.children[0].id}\u0000gone`), other.children[0]);
  assert.equal(findNode(root, 'nowhere'), root);
  assert.deepEqual(pathTo(other.children[0]).map(node => node.key), ['all', '\u0001other', 't7']);
});

test('the zoomed slice fills the circle and its children share it; Fill sweeps toward the whole', () => {
  const root = groupTree([item(['a', 'x'], 30, { final: 60 }), item(['a', 'y'], 10, { final: 20 }), item(['b', 'z'], 40, { final: 40 })], ROOT);
  const all = arcTargets(root, root, 'share');
  const [a, b] = root.children;
  assert.ok(Math.abs(all.get(a.id)!.a1 - Math.PI) < 1e-9);
  assert.equal(all.get(a.id)!.ring, 1);
  assert.equal(all.get(a.children[0].id)!.ring, 2);
  assert.equal(all.get(a.children[0].id)!.shown, true);
  const zoomed = arcTargets(root, a, 'share');
  assert.ok(Math.abs(zoomed.get(a.children[0].id)!.a1 - Math.PI * 1.5) < 1e-9);
  assert.equal(zoomed.get(a.children[0].id)!.ring, 1);
  // Outside the zoomed slice, nothing shows.
  assert.equal(zoomed.get(b.id)!.shown, false);
  assert.equal(zoomed.get(a.id)!.shown, false);
  // Fill: 80 of the 120 there will be.
  assert.ok(Math.abs(sweepOf(root, 'fill') - Math.PI * 2 * 80 / 120) < 1e-9);
  const filled = arcTargets(root, root, 'fill');
  assert.ok(Math.abs(filled.get(b.id)!.a1 - sweepOf(root, 'fill')) < 1e-9);
  // Three rings out is not shown with two rings.
  const deep = groupTree([item(['a', 'b', 'c'], 1)], ROOT);
  assert.equal(arcTargets(deep, deep, 'share').get(walk(deep).at(-1)!.id)!.shown, false);
});

test('5,000 categories make one Other level, with grouping and folding in about linear time', () => {
  const items = Array.from({ length: 5_000 }, (_, index) => item(['parent', `t${index}`], 5_000 - index, { exact: BigInt(5_000 - index), counted: [`p${index}`] }));
  const root = foldTree(groupTree(items, ROOT));
  const parent = root.children[0];
  const other = parent.children.at(-1)!;
  assert.equal(parent.children.length, 8);
  assert.equal(other.other, true);
  assert.equal(other.children.length, 4_993);
  assert.equal(walk(root).filter(node => node.other).length, 1);
  assert.equal(Math.max(...walk(root).map(node => node.depth)), 3);
  assert.ok(other.children.every(child => !child.other && child.parent === other));
  assert.equal(root.count, 5_000);
  assert.equal(root.exact, 12_502_500n);
  // Warm up, then take the median so a load spike cannot fail the budget alone.
  const times: number[] = [];
  for (let round = 0; round < 10; round++) {
    const started = performance.now();
    foldTree(groupTree(items, ROOT));
    if (round >= 3) times.push(performance.now() - started);
  }
  const median = times.sort((a, b) => a - b)[3];
  assert.ok(median < 100, `5,000 categories grouped and folded in ${median.toFixed(1)} ms`);
  assertLinear('sunburst categories', count => {
    const items = Array.from({ length: count }, (_, index) => item(['parent', `t${index}`], count - index));
    return () => foldTree(groupTree(items, ROOT));
  });
});

test('Other opens and folds its parts only on the zoom path, which survives playback', () => {
  const items = Array.from({ length: 30 }, (_, index) => item([`t${index}`, 'place'], 30 - index, { exact: BigInt(30 - index) }));
  const root = foldTree(groupTree(items, ROOT));
  const other = root.children.at(-1)!;
  const hidden = other.children[0];
  assert.equal(arcTargets(root, root, 'share').get(hidden.id)!.shown, false);
  assert.equal(sliceColours(root, Array(8).fill('#ff0000'), { light: '#ffffff', dark: '#000000' }).has(hidden.id), false);
  const focus = foldView(root, [other.id]);
  assert.equal(focus, other);
  assert.equal(focus.children.length, 8);
  const nested = focus.children.at(-1)!;
  assert.equal(nested.other, true);
  assert.ok(nested.children.every(child => !child.other));
  const rings = keyRings(root, arcTargets(root, focus, 'share'));
  assert.equal(rings[0].length, 8);
  assert.equal(arcTargets(root, focus, 'share').get(hidden.id)!.shown, true);
  assert.ok(rings[1].every(node => node.parent !== nested));
  assert.equal(focus.value, 276);
  assert.equal(focus.exact, 276n);
  const deeper = foldView(root, [other.id, nested.id]);
  const part = deeper.children[0];
  const path = pathTo(part).slice(1).map(node => node.id);
  const rebuilt = foldTree(groupTree(items.map(item => ({ ...item, value: item.value! / 2, final: item.value })), ROOT));
  const restored = foldView(rebuilt, path);
  assert.equal(restored.id, part.id);
  assert.deepEqual(pathTo(restored).slice(1).map(node => node.id), path);
  assert.equal(walk(rebuilt).filter(node => node.other).length, walk(root).filter(node => node.other).length);
  const back = foldTree(groupTree(items, ROOT));
  foldView(back, [other.id]);
  assert.equal(walk(back).filter(node => node.other).length, 2);
});

test('the innermost ring takes the palette in order and outer rings shade their parent', () => {
  const root = groupTree([item(['a', 'x'], 3), item(['a', 'y'], 2), item(['b', 'z'], 1)], ROOT);
  const colours = sliceColours(root, ['#ff0000', '#00ff00'], { light: '#ffffff', dark: '#000000' });
  const [a, b] = root.children;
  assert.equal(colours.get(a.id), '#ff0000');
  assert.equal(colours.get(b.id), '#00ff00');
  assert.equal(colours.get(a.children[0].id), '#ff0000');
  assert.notEqual(colours.get(a.children[1].id), '#ff0000');
  // Zoomed in, a's children become the innermost ring and take the palette.
  const zoomed = sliceColours(a, ['#0000ff', '#00ff00'], { light: '#ffffff', dark: '#000000' });
  assert.equal(zoomed.get(a.children[1].id), '#00ff00');
  assert.equal(zoomed.has(b.id), false);
});

test('a point finds the centre or the slice drawn under it, and labels fit only where there is room', () => {
  const g = sunGeometry(200, 200);
  assert.equal(g.cx, 100);
  assert.ok(g.hole < g.radius);
  const slices = [
    { id: 'right', a0: 0, a1: Math.PI, inner: g.hole, outer: g.hole + g.ring, alpha: 1 },
    { id: 'fading', a0: Math.PI, a1: Math.PI * 2, inner: g.hole, outer: g.hole + g.ring, alpha: 0.2 },
  ];
  assert.equal(sliceAt(slices, g, 100, 100), 'centre');
  const middle = g.hole + g.ring / 2;
  assert.equal(sliceAt(slices, g, 100 + middle, 100), 'right');
  assert.equal(sliceAt(slices, g, 100 - middle, 100), null);
  assert.equal(sliceAt(slices, g, 199.9, 199.9), null);
  assert.equal(labelFits({ a0: 0, a1: Math.PI / 2, inner: 40, outer: 70 }, 40, 11), true);
  assert.equal(labelFits({ a0: 0, a1: 0.1, inner: 40, outer: 70 }, 40, 11), false);
  assert.equal(labelFits({ a0: 0, a1: Math.PI, inner: 40, outer: 50 }, 20, 11), false);
  // Codex round 4: a 180 px label on a 120-degree outer slice (middle radius 117.6)
  // reaches radius 148 at its ends, past the outer 140, though its arc is long.
  assert.equal(labelFits({ a0: 0, a1: Math.PI * 2 / 3, inner: 95.2, outer: 140 }, 180, 11), false);
  assert.equal(labelFits({ a0: 0, a1: Math.PI * 2 / 3, inner: 95.2, outer: 140 }, 60, 11), true);
});

test('the keys go round a ring, out to a child and back in to its parent', () => {
  const root = groupTree([item(['a', 'x'], 3), item(['a', 'y'], 2), item(['b', 'z'], 4)], ROOT);
  const rings = keyRings(root, arcTargets(root, root, 'share'));
  assert.deepEqual(rings.map(ring => ring.map(node => node.key)), [['a', 'b'], ['x', 'y', 'z']]);
  const a = keyedSlice('ArrowRight', null, rings)!;
  assert.equal(a.key, 'a');
  assert.equal(keyedSlice('ArrowRight', a.id, rings)!.key, 'b');
  assert.equal(keyedSlice('ArrowLeft', a.id, rings)!.key, 'b');
  const x = keyedSlice('ArrowDown', a.id, rings)!;
  assert.equal(x.key, 'x');
  assert.equal(keyedSlice('End', x.id, rings)!.key, 'z');
  assert.equal(keyedSlice('ArrowUp', x.id, rings), a);
  assert.equal(keyedSlice('ArrowUp', a.id, rings), a);
  assert.equal(keyedSlice('ArrowDown', 'gone', rings)!.key, 'a');
});

test('a slice tooltip gives its exact amount, shares, count and what is not counted, fullest first', () => {
  const root = groupTree([
    item(['a', 'x'], 30, { counted: ['p1', 'p2'], unknown: { gap: 2, late: 1 } }),
    item(['a', 'y'], 10, { counted: ['p3'] }),
    item(['r', 'z'], null, { note: 'Not measured here.', counted: ['p4', 'p5', 'p6', 'p7'] }),
  ], ROOT);
  const text = { format: (value: number) => value.toFixed(0), unit: 'IC', valueLabel: 'Spent', countLabel: 'Projects', unknownLabel: (reason: string) => `Reason ${reason}` };
  const [a, r] = root.children;
  const x = a.children[0];
  const [full, compact] = sliceTooltip(x, root, text);
  assert.equal(full.title, 'A › X');
  assert.deepEqual(full.rows.map(row => [row.label, row.value]), [
    ['Spent', '30 IC'], ['Of A', '75%'], ['Of All', '75%'], ['Projects', '2'], ['Not counted', '3'], ['Reason gap', '2'], ['Reason late', '1'],
  ]);
  assert.deepEqual(compact.rows.map(row => row.label), ['Spent', 'Of A', 'Of All', 'Projects', 'Not counted']);
  const [unmeasured] = sliceTooltip(r, root, text);
  assert.deepEqual(unmeasured.rows.map(row => [row.label, row.value]), [['Spent', 'Not measured'], ['Projects', '4']]);
  assert.equal(unmeasured.note, 'Not measured here.');
  assert.equal(sunburstSummary(root, text), 'All: 40 IC, 3 not counted; A: 40 IC, 100%, 3 not counted; R: not measured');
  // Zoomed into a slice that is not measured, the read-out says so, as the centre does.
  assert.equal(sunburstSummary(r, text), 'R: not measured; Z: not measured');
});

test('shares are written to a tenth, a sliver as under a tenth', () => {
  assert.equal(shareText(1, 3), '33.3%');
  assert.equal(shareText(1, 5000), '<0.1%');
  assert.equal(shareText(0, 5), '0%');
  assert.equal(shareText(1, 0), '—');
});

test('a part with nothing counted yet but parts not counted stays listed, with its uncertainty', () => {
  const root = groupTree([item(['a'], 50), item(['gap'], 0, { final: 0, unknown: { notRecorded: 1 } })], ROOT);
  const gap = root.children.find(child => child.key === 'gap')!;
  assert.equal(gap.unmeasured, false);
  assert.equal(listed(gap), true);
  const text = { format: (value: number) => String(value), unit: 'IC', valueLabel: 'Spent' };
  assert.equal(sunburstSummary(root, text), 'All: 50 IC, 1 not counted; A: 50 IC, 100%; GAP: nothing counted, 1 not counted');
  // A part with nothing at all is not listed.
  assert.equal(listed(groupTree([item(['a'], 0, { final: 0 })], ROOT).children[0]), false);
});

test('an item not yet present adds only its whole amount: order, colours and Fill are the end\'s', () => {
  // A spends its last 100 by the hour shown; B appears later and spends 900.
  const at = (bPresent: boolean, aValue: number, bValue: number | null) => groupTree([
    item(['a'], aValue, { final: 100 }),
    item(['b'], bValue, { final: 900, present: bPresent }),
  ], ROOT);
  const early = at(false, 100, null);
  assert.equal(early.value, 100);
  assert.equal(early.final, 1000);
  assert.ok(Math.abs(sweepOf(early, 'fill') - Math.PI * 2 * 0.1) < 1e-9);
  const [b] = early.children;
  assert.equal(b.key, 'b');
  assert.equal(b.present, false);
  assert.equal(listed(b), false);
  const late = at(true, 100, 900);
  assert.deepEqual(early.children.map(child => child.key), late.children.map(child => child.key));
  const palette = ['#ff0000', '#00ff00'];
  const shades = { light: '#ffffff', dark: '#000000' };
  const a = (tree: typeof early) => sliceColours(tree, palette, shades).get(tree.children.find(child => child.key === 'a')!.id);
  assert.equal(a(early), a(late));
});

test('equal whole amounts keep one order by data key, whatever the amounts so far', () => {
  const tree = (x: number, y: number) => groupTree([item(['y'], y, { final: 50 }), item(['x'], x, { final: 50 })], ROOT);
  assert.deepEqual(tree(10, 40).children.map(child => child.key), ['x', 'y']);
  assert.deepEqual(tree(40, 10).children.map(child => child.key), ['x', 'y']);
  // Folding into Other follows the same order.
  const many = (bump: number) => foldTree(groupTree(Array.from({ length: 10 }, (_, index) =>
    item([`k${index}`], index === 9 ? bump : 1, { final: 5 })), ROOT), 8);
  assert.deepEqual(many(0).children.at(-1)!.children.map(child => child.key), many(4).children.at(-1)!.children.map(child => child.key));
});

test('exact amounts add up without rounding through a number', () => {
  const root = groupTree([item(['a'], 9007199254741499, { exact: 9007199254741499n }), item(['b'], 2, { exact: 2n })], ROOT);
  assert.equal(root.exact, 9007199254741501n);
  assert.equal(groupTree([item(['a'], 1, { exact: 1n }), item(['b'], 2)], ROOT).exact, null);
  assert.equal(groupTree([item(['r'], null, { note: 'n' })], ROOT).children[0].exact, null);
});

test('a colour glide ends exactly, however small each frame\'s step', () => {
  // Smooth motion at 24 h/s with 16 ms frames: about 8.5% a frame.
  const amount = 1 - Math.exp(-16 / 180);
  const target = { r: 0x39, g: 0x87, b: 0xe5 };
  let colour = { r: 0x3e, g: 0x82, b: 0xe0 };
  let frames = 0;
  while ((colour.r !== target.r || colour.g !== target.g || colour.b !== target.b) && frames < 1000) {
    colour = approachColour(colour, target, amount);
    frames++;
  }
  assert.deepEqual(colour, target);
  assert.ok(frames < 60, `${frames} frames`);
  assert.deepEqual(approachColour(colour, target, amount), target);
  assert.deepEqual(approachColour({ r: 0, g: 0, b: 0 }, target, 1), target);
});

test('folding goes by whole amounts, so a type that is repaired first and built later folds the same all along', () => {
  const tree = (built: boolean) => foldTree(groupTree([
    ...Array.from({ length: 8 }, (_, index) => item([`t${index}`], 10 + index, { final: 100 + index })),
    // Type z: a repair (not measured) now; later construction, not present yet, of 50.
    item(['z'], null, { note: 'n' }),
    item(['z'], built ? 50 : null, { final: 50, present: built }),
  ], ROOT), 8);
  const other = (root: ReturnType<typeof tree>) => root.children.find(child => child.other)!.children.map(child => child.key);
  assert.deepEqual(other(tree(false)), other(tree(true)));
  assert.ok(other(tree(false)).includes('z'));
});

test('parts that are never drawn take no palette slot', () => {
  const root = groupTree([item(['r'], null, { note: 'n' }), item(['u'], 0, { final: 0, unknown: { gap: 1 } }), item(['x'], 5), item(['y'], 3)], ROOT);
  const colours = sliceColours(root, ['#ff0000', '#00ff00'], { light: '#ffffff', dark: '#000000' });
  const id = (key: string) => root.children.find(child => child.key === key)!.id;
  assert.equal(colours.get(id('x')), '#ff0000');
  assert.equal(colours.get(id('y')), '#00ff00');
  assert.equal(colours.has(id('r')), false);
  assert.equal(colours.has(id('u')), false);
});

test('Other is not measured when only an unmeasured part of it is present', () => {
  // Nine types (the new-or-repair level switched off); only a lower-ranked type's
  // repair has appeared, and it falls in Other.
  const items = [
    ...Array.from({ length: 9 }, (_, index) => item([`t${index}`], null, { final: 100 - index, present: false })),
    item(['t8'], null, { note: 'Repairs are not measured.' }),
  ];
  const root = foldTree(groupTree(items, ROOT), 8);
  const other = root.children.find(child => child.other)!;
  assert.equal(other.present, true);
  assert.equal(other.unmeasured, true);
  assert.equal(listed(other), true);
  assert.equal(other.exact, null);
  assert.deepEqual(other.notes, ['Repairs are not measured.']);
  // With a measured part present, Other is measured and sums it exactly.
  const measured = foldTree(groupTree([...items, item(['t7'], 5, { exact: 5n, final: 0 })], ROOT), 8);
  const withAmount = measured.children.find(child => child.other)!;
  assert.equal(withAmount.unmeasured, false);
  assert.equal(withAmount.exact, 5n);
});

test('a project in two merged categories counts once', () => {
  // With the new-or-repair level off, a project that repaired and then built is
  // in both of its type's items.
  const root = groupTree([
    item(['repair', 'civ'], null, { note: 'n', counted: ['9'] }),
    item(['new', 'civ'], 40, { counted: ['9', '10'] }),
  ], ROOT, [false, true]);
  assert.equal(root.children[0].count, 2);
  assert.equal(root.count, 2);
});

test('a merged category with unmeasured parts and nothing counted stays listed', () => {
  // With the new-or-repair level off, a type with a repair and a new project that
  // has spent nothing counted yet.
  const root = groupTree([
    item(['repair', 'civ'], null, { note: 'Repairs are not measured.' }),
    item(['new', 'civ'], 0),
  ], ROOT, [false, true]);
  const civ = root.children[0];
  assert.equal(civ.unmeasured, false);
  assert.equal(civ.partlyUnmeasured, true);
  assert.equal(listed(civ), true);
  const text = { format: (value: number) => String(value), unit: 'IC', valueLabel: 'Spent' };
  assert.equal(sunburstSummary(root, text), 'All: not measured; CIV: not measured');
  // Its tooltip says so too, not "0 IC".
  const [tip] = sliceTooltip(civ, root, text);
  assert.deepEqual(tip.rows[0], { label: 'Spent', value: 'Not measured', tone: 'muted' });
});

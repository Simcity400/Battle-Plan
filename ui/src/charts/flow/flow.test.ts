import assert from 'node:assert/strict';
import test from 'node:test';
import { flowCapacity, FlowMotion, flowStops, linkAt, nodeAt, ribbonTop, slotOrder } from './layout.ts';
import type { FlowGeometry, FlowLink, FlowNode } from './layout';

const node = (id: string, side: 'source' | 'target', extra: Partial<FlowNode> = {}): FlowNode => ({ id, side, label: id, color: '#888', order: id, ...extra });
const link = (source: string, target: string, value: number, order = ''): FlowLink => ({ id: `${source}>${target}${order}`, source, target, value, color: '#123', order });
// Two slots a side: 114 px less one 10 px gap and each node's 2 px minimum leave 100.
const geometry: FlowGeometry = { left: 100, right: 300, top: 0, bottom: 114, nodeWidth: 10, gap: 10 };
const nodes = [node('A', 'source'), node('B', 'source'), node('X', 'target'), node('Y', 'target')];

test('slots put the largest first, ties by key and a last node at the end', () => {
  const sizes = new Map([['A', 1], ['B', 5], ['C', 5], ['O', 99]]);
  const order = slotOrder([node('C', 'source'), node('O', 'source', { last: true }), node('A', 'source'), node('B', 'source'), node('X', 'target')], sizes, 'source');
  assert.deepEqual(order.map(item => item.id), ['B', 'C', 'A', 'O']);
});

test('snapped, nodes fill the height in proportion and ribbons stack inside them in a fixed order', () => {
  const motion = new FlowMotion();
  const frame = motion.step({ nodes, links: [link('A', 'Y', 30), link('A', 'X', 10), link('B', 'X', 60)] }, geometry, 16, 100, true);
  assert.equal(frame.settled, true);
  const box = (id: string) => frame.nodes.find(item => item.id === id)!;
  // 1 px per unit, and each node 2 px more.
  assert.deepEqual([box('B').y, box('B').height, box('A').y, box('A').height], [0, 62, 72, 42]);
  assert.deepEqual([box('X').y, box('X').height, box('Y').y, box('Y').height], [0, 72, 82, 32]);
  const shape = (id: string) => frame.links.find(item => item.id === id)!;
  // Inside A, the ribbon to X comes before the one to Y by key, whatever their sizes.
  assert.deepEqual([shape('A>X').y0, shape('A>Y').y0], [72, 82]);
  // Inside X, A's ribbon comes before B's.
  assert.deepEqual([shape('A>X').y1, shape('B>X').y1], [0, 10]);
  assert.equal(shape('B>X').width, 60);
});

test('a campaign scale keeps widths as amounts while the total changes', () => {
  const motion = new FlowMotion();
  const frame = motion.step({ nodes, links: [link('A', 'X', 25)] }, { ...geometry, scale: 100, slots: 2 }, 16, 0, true);
  // 100 fills the 100 px left after one gap and two minimum nodes.
  assert.equal(frame.links[0].width, 25);
});

test('many nodes shrink the gaps, keep a minimum height and never run past the bottom', () => {
  // 40 a side in 200 px: the 10 px gaps alone would not fit.
  const many = Array.from({ length: 40 }, (_, index) => [node(`S${index}`, 'source'), node(`T${index}`, 'target')]).flat();
  const links = Array.from({ length: 40 }, (_, index) => link(`S${index}`, `T${index}`, index === 0 ? 1000 : 0.001));
  const frame = new FlowMotion().step({ nodes: many, links }, { ...geometry, bottom: 200 }, 16, 0, true);
  assert.equal(frame.nodes.length, 80);
  assert.ok(frame.nodes.every(box => box.height >= 2 && box.y + box.height <= 200 + 1e-9), 'every node drawn inside the height');
  // How many a side a widget should ask for at that height, one label row each.
  assert.equal(flowCapacity(200, 16), 12);
  assert.equal(flowCapacity(10, 16), 2);
});

test('ribbons grow in, shrink away and nodes glide to their new slots', () => {
  const motion = new FlowMotion();
  motion.step({ nodes, links: [link('A', 'X', 40), link('B', 'Y', 60)] }, geometry, 16, 100, true);
  // A new ribbon grows from nothing; a ribbon no longer wanted shrinks, then goes.
  let frame = motion.step({ nodes, links: [link('A', 'X', 40), link('A', 'Y', 20)] }, geometry, 16, 100);
  const grown = frame.links.find(item => item.id === 'A>Y')!;
  const going = frame.links.find(item => item.id === 'B>Y')!;
  assert.ok(grown.width > 0 && grown.width < 20 * 100 / 100);
  assert.ok(going.width > 0);
  assert.equal(frame.settled, false);
  // A was the smaller source; now it is the only one and glides up to the top.
  const aBefore = frame.nodes.find(item => item.id === 'A')!.y;
  assert.ok(aBefore > 0);
  for (let step = 0; step < 400; step++) frame = motion.step({ nodes, links: [link('A', 'X', 40), link('A', 'Y', 20)] }, geometry, 16, 100);
  assert.equal(frame.settled, true);
  assert.equal(frame.links.some(item => item.id === 'B>Y'), false);
  assert.equal(frame.nodes.some(item => item.id === 'B'), false);
  assert.equal(frame.nodes.find(item => item.id === 'A')!.y, 0);
});

test('a snap jumps across a gap instead of gliding', () => {
  const motion = new FlowMotion();
  motion.step({ nodes, links: [link('A', 'X', 40)] }, geometry, 16, 100, true);
  const frame = motion.step({ nodes, links: [link('B', 'Y', 10)] }, geometry, 16, 100, true);
  assert.deepEqual(frame.links.map(item => item.id), ['B>Y']);
  assert.equal(frame.settled, true);
});

test('the point on a ribbon or a node finds it, and a thin ribbon is still easy to point at', () => {
  const shape = { x0: 0, x1: 100, y0: 0, y1: 50 };
  assert.ok(Math.abs(ribbonTop(shape, 0)!) < 1e-6);
  assert.equal(Math.round(ribbonTop(shape, 50)! * 1000) / 1000, 25);
  assert.ok(Math.abs(ribbonTop(shape, 100)! - 50) < 1e-6);
  assert.equal(ribbonTop(shape, 101), null);
  const motion = new FlowMotion();
  const frame = motion.step({ nodes, links: [link('A', 'X', 99.9), link('B', 'Y', 0.1)] }, geometry, 16, 0, true);
  const middle = (id: string) => { const item = frame.links.find(each => each.id === id)!; return ribbonTop(item, 200)! + item.width / 2; };
  assert.equal(linkAt(frame.links, 200, middle('A>X'))?.id, 'A>X');
  // 0.1 px wide, yet within a few pixels of it points at it.
  assert.equal(linkAt(frame.links, 200, middle('B>Y') + 2)?.id, 'B>Y');
  assert.equal(linkAt(frame.links, 50, 10), null);
  assert.equal(nodeAt(frame.nodes, 95, 5)?.id, 'A');
  assert.equal(nodeAt(frame.nodes, 50, 5), null);
});

test('the keyboard stops at each source node and its ribbons, then each target node', () => {
  const motion = new FlowMotion();
  const frame = motion.step({ nodes, links: [link('A', 'Y', 30), link('A', 'X', 10), link('B', 'X', 60)] }, geometry, 16, 0, true);
  assert.deepEqual(flowStops(frame).map(stop => `${stop.kind}:${stop.id}`),
    ['node:B', 'link:B>X', 'node:A', 'link:A>X', 'link:A>Y', 'node:X', 'node:Y']);
});

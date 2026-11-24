import test from 'node:test';
import assert from 'node:assert/strict';
import { ARC_GAP, chordGap, chordStops, chordSurface, hitChord, layoutChord, ribbonContains, stepChord } from './layout.ts';
import type { ChordData } from './layout.ts';
import { ChordMotion } from './motion.ts';
import { foldChord } from './fold.ts';
import { checkChord } from './check.ts';
import { ChordFrames } from './frame.ts';
import { chordTotals } from './totals.ts';
import { deepFreeze } from '../../testFrozen.ts';

const node = (id: string) => ({ id, label: id, color: 'recorded colour', flag: 'recorded flag' });
const graph: ChordData = deepFreeze({ nodes: [node('mod country with spaces'), node('removed:replacement'), node('dynamic | name')], flows: [
  { id: 'forward', source: 'mod country with spaces', target: 'removed:replacement', value: 30 },
  { id: 'reverse', source: 'removed:replacement', target: 'mod country with spaces', value: 10 },
  { id: 'third', source: 'dynamic | name', target: 'removed:replacement', value: 20 },
] });
const near = (a: number, b: number) => assert.ok(Math.abs(a - b) < 1e-9, `${a} != ${b}`);

test('arc spans count directed in + out and each flow consumes matching segments', () => {
  const layout = layoutChord(graph);
  near(layout.arcs[0].start, -Math.PI / 2);
  assert.deepEqual(layout.arcs.map(arc => [arc.incoming, arc.outgoing]), [[10, 30], [50, 10], [0, 20]]);
  const spans = layout.arcs.map(arc => arc.end - arc.start);
  near(spans[1] / spans[2], 3);
  near(spans.reduce((a, b) => a + b, 0) + 3 * ARC_GAP, 2 * Math.PI);
  for (const ribbon of layout.ribbons) near(ribbon.sourceEnd - ribbon.sourceStart, ribbon.targetEnd - ribbon.targetStart);
  near(layout.ribbons[0].sourceEnd, layout.ribbons[1].targetStart);
});

test('zero flows consume no arc and folding retains internal flows as self ribbons', () => {
  const folded = foldChord(graph, new Set(['mod country with spaces']), node('other'));
  assert.deepEqual(folded.nodes.get('other'), ['removed:replacement', 'dynamic | name']);
  assert.equal(folded.data.flows.reduce((sum, flow) => sum + flow.value, 0), 60);
  assert.equal([...folded.flows.values()].flat().length, graph.flows.length);
  const layout = layoutChord(folded.data), self = layout.ribbons.find(ribbon => ribbon.flow.source === ribbon.flow.target)!;
  near(self.sourceEnd, self.targetStart);
  const empty = layoutChord({ nodes: graph.nodes, flows: graph.flows.map(flow => ({ ...flow, value: 0 })) });
  assert.equal(empty.ribbons.length, 0);
  assert.ok(empty.arcs.every(arc => arc.start === arc.end));
});

test('opaque identifiers remain distinct when they contain delimiters or JSON syntax', () => {
  const data: ChordData = { nodes: [node('a|b'), node('c'), node('a'), node('b|c'), node('["other"]')], flows: [
    { id: 'first', source: 'a|b', target: 'c', value: 1 }, { id: 'second', source: 'a', target: 'b|c', value: 2 },
  ] };
  const folded = foldChord(data, new Set(data.nodes.map(node => node.id)), node('aggregate'));
  assert.equal(new Set(folded.data.flows.map(flow => flow.id)).size, 2);
  assert.equal(folded.data.nodes[0].flag, 'recorded flag');
  assert.equal(folded.data.nodes[0].color, 'recorded colour');
  assert.throws(() => foldChord(data, new Set(), node('a')));
});

test('a new graph grows from zero; values, gaps and order glide through a value change', () => {
  const motion = new ChordMotion(); motion.set(graph);
  const first = motion.advance(16, 180);
  assert.ok(first.moving && first.ringPresence > 0 && first.ringPresence < 1);
  const entering = layoutChord(first.data, motion.presence, first.ringPresence);
  assert.ok(entering.arcs.reduce((sum, arc) => sum + arc.end - arc.start, 0) < 2 * Math.PI / 2);
  motion.advance(16, 0);
  const next = { nodes: [...graph.nodes].reverse(), flows: [...graph.flows].reverse().map(flow => ({ ...flow, value: flow.id === 'forward' ? 0 : flow.value * 2 })) };
  motion.set(next); const glide = motion.advance(16, 180);
  assert.deepEqual(glide.data.nodes.map(node => node.id), graph.nodes.map(node => node.id));
  assert.deepEqual(glide.data.flows.map(flow => flow.id), graph.flows.map(flow => flow.id));
  assert.ok(glide.data.flows[0].value > 0 && glide.data.flows[0].value < 30);
  assert.equal(next.flows.at(-1)!.value, 0, 'presentation never changes exact input');
  const settled = motion.advance(16, 0);
  assert.equal(settled.moving, false); assert.ok(!settled.data.flows.some(flow => flow.id === 'forward'));
  motion.set({ nodes: [], flows: [] }); const leaving = motion.advance(16, 180);
  assert.ok(leaving.ringPresence > 0 && leaving.ringPresence < 1);
  assert.ok(leaving.data.flows.length > 0);
  motion.clear(); assert.deepEqual(motion.advance(16, 180).data, { nodes: [], flows: [] });
});

test('clear then set snaps the first layout, even after an empty gap frame', () => {
  const motion = new ChordMotion(); motion.set(graph); motion.advance(16, 0);
  motion.clear(); motion.advance(16, 180);
  motion.set(graph);
  const returned = motion.advance(16, 180);
  assert.equal(returned.moving, false); assert.equal(returned.ringPresence, 1);
  assert.deepEqual(returned.data, graph);
  assert.deepEqual(layoutChord(returned.data, motion.presence, returned.ringPresence), layoutChord(graph));
  motion.set({ ...graph, flows: graph.flows.map(flow => ({ ...flow, value: flow.value * 2 })) });
  assert.equal(motion.advance(16, 180).moving, true, 'later uninterrupted changes still glide');
});

test('frame acceptance is atomic, retryable and snaps at a new segment', () => {
  const frames = new ChordFrames(); frames.accept({ data: graph, segment: 'a' }); frames.motion.advance(1, 0);
  const refused = deepFreeze({ ...graph, flows: [{ ...graph.flows[0], target: 'missing' }] });
  assert.throws(() => frames.accept({ data: refused, segment: 'a' }), /Unknown chord endpoint/);
  assert.equal(frames.data, graph, 'tooltips keep the accepted exact graph');
  assert.deepEqual(frames.motion.advance(1, 0).data, graph, 'refusal leaves motion intact');
  const repaired = deepFreeze({ ...refused, flows: [{ ...refused.flows[0], target: graph.flows[0].target }] });
  frames.accept({ data: repaired, segment: 'a' });
  assert.equal(frames.data, repaired, 'a replacement repairs refused data');
  frames.accept({ data: graph, segment: 'b' });
  assert.equal(frames.motion.advance(16, 180).moving, false);
  frames.accept({ data: null }); assert.equal(frames.data, null);
});

test('any target and its departing/entering mixture retain positive arcs within one ring', () => {
  const data = (prefix: string, count: number): ChordData => ({
    nodes: Array.from({ length: count }, (_, i) => node(`${prefix}${i}`)),
    flows: Array.from({ length: count }, (_, i) => ({ id: `${prefix}${i}`, source: `${prefix}${i}`, target: `${prefix}${i}`, value: 1 })),
  });
  const motion = new ChordMotion(); motion.set(data('a', 500)); motion.advance(1, 0);
  motion.set(data('b', 500));
  const mixed = motion.advance(16, 180);
  assert.equal(mixed.data.nodes.length, 1000);
  const layout = layoutChord(mixed.data, motion.presence, mixed.ringPresence);
  assert.ok(layout.arcs.every(arc => arc.end > arc.start));
  const gap = chordGap(layout.arcs.reduce((sum, arc) => sum + arc.presence, 0));
  near(layout.arcs.reduce((sum, arc) => sum + arc.end - arc.start + gap * arc.presence, 0), 2 * Math.PI);
});

test('exact node totals include both directions and count a self-flow in each', () => {
  const flows = [{ source: 'mod | country', target: 'other', raw: 9223372036854775807n },
    { source: 'mod | country', target: 'mod | country', raw: 1n }];
  const totals = chordTotals(flows, f => f.source, f => f.target, f => f.raw);
  assert.deepEqual(totals.get('mod | country'), { incoming: 1n, outgoing: 9223372036854775808n });
  assert.deepEqual(totals.get('other'), { incoming: 9223372036854775807n, outgoing: 0n });
});

test('a folded node appearing and leaving retains total mass without changing old order', () => {
  const motion = new ChordMotion(), other = node('fold');
  motion.set(foldChord(graph, new Set(graph.nodes.map(node => node.id)), other).data); motion.advance(1, 0);
  motion.set(foldChord(graph, new Set([graph.nodes[0].id]), other).data);
  const step = motion.advance(16, 180);
  near(step.data.flows.reduce((sum, flow) => sum + flow.value, 0), 60);
  assert.deepEqual(step.data.nodes.slice(0, 3), graph.nodes);
  assert.ok((motion.presence.get('fold') ?? 0) > 0);
  motion.set(foldChord(graph, new Set(graph.nodes.map(node => node.id)), other).data);
  assert.ok(motion.advance(16, 180).data.flows.some(flow => flow.source === 'fold' || flow.target === 'fold'));
});

test('settled departures leave frame work, but returning identities keep their order', () => {
  const motion = new ChordMotion();
  for (let index = 0; index < 5_000; index++) {
    motion.set({ nodes: [node(`node ${index}`)], flows: [{ id: `self ${index}`, source: `node ${index}`, target: `node ${index}`, value: 1 }] });
    const frame = motion.advance(1, 0);
    assert.equal(frame.data.nodes.length, 1); assert.equal(frame.data.flows.length, 1);
    assert.equal(motion.presence.size, 1);
  }
  motion.set({ nodes: [node('node 4'), node('node 2')], flows: [
    { id: 'self 4', source: 'node 4', target: 'node 4', value: 1 },
    { id: 'self 2', source: 'node 2', target: 'node 2', value: 1 },
  ] });
  const frame = motion.advance(1, 0);
  assert.deepEqual(frame.data.nodes.map(node => node.id), ['node 2', 'node 4']);
  assert.deepEqual(frame.data.flows.map(flow => flow.id), ['self 2', 'self 4']);
  assert.throws(() => motion.set({ nodes: [node('node 4')], flows: [{ id: 'self 2', source: 'node 4', target: 'node 4', value: 1 }] }));
  assert.deepEqual(motion.advance(1, 0).data, frame.data, 'a refused update leaves the prior graph intact');
});

test('hit tests reach arcs and quadratic ribbons, reject the ring gap and outside', () => {
  const layout = layoutChord({ nodes: [node('source'), node('target')], flows: [{ id: 'flow', source: 'source', target: 'target', value: 1 }] });
  const surface = { x: 150, y: 150, radius: 100 };
  assert.deepEqual(hitChord(layout, surface, 150, 150), { kind: 'flow', id: 'flow' });
  assert.equal(ribbonContains(layout.ribbons[0], 200, 0, 92), false);
  const mid = (layout.arcs[0].start + layout.arcs[0].end) / 2;
  assert.deepEqual(hitChord(layout, surface, 150 + Math.cos(mid) * 98, 150 + Math.sin(mid) * 98), { kind: 'node', id: 'source' });
  const gap = layout.arcs[0].end + ARC_GAP / 2;
  assert.equal(hitChord(layout, surface, 150 + Math.cos(gap) * 98, 150 + Math.sin(gap) * 98), null);
  assert.equal(hitChord(layout, surface, 400, 150), null);
  assert.ok(chordSurface(50, 40).radius >= 0);
  assert.deepEqual(chordSurface(320, 280), { x: 120, y: 144, radius: 44 }, 'style-6 placement also holds in a narrow card');
});

test('keyboard navigation reaches every ribbon and arc, preserving identity on updates', () => {
  const stops = chordStops(layoutChord(graph));
  assert.equal(stops.length, 6);
  let current = stepChord(stops, null, 1);
  assert.deepEqual(current, stops[0]);
  for (let index = 1; index < stops.length; index++) { current = stepChord(stops, current, 1); assert.deepEqual(current, stops[index]); }
  assert.deepEqual(stepChord(stops, current, 1), stops[0]);
  assert.deepEqual(stepChord(stops, null, -1), stops.at(-1));
  assert.equal(stepChord([], current, 1), null);
  assert.deepEqual(chordStops(layoutChord(graph, undefined, 0)), [], 'zero-width items have no keyboard position');
});

test('malformed or ambiguous geometry is refused, never silently dropped', () => {
  assert.throws(() => checkChord({ ...graph, nodes: [...graph.nodes, graph.nodes[0]] }));
  assert.throws(() => checkChord({ ...graph, flows: [...graph.flows, graph.flows[0]] }));
  assert.throws(() => checkChord({ ...graph, flows: [{ ...graph.flows[0], target: 'absent' }] }));
  assert.throws(() => foldChord({ ...graph, nodes: [...graph.nodes, graph.nodes[0]] }, new Set(), node('other')));
  assert.throws(() => foldChord({ ...graph, flows: [...graph.flows, graph.flows[0]] }, new Set(), node('other')));
  for (const value of [-1, NaN, Infinity, Number.MAX_VALUE]) assert.throws(() => checkChord({ ...graph, flows: [{ ...graph.flows[0], value }] }));
  const motion = new ChordMotion(); motion.set(graph);
  assert.throws(() => motion.set({ ...graph, flows: [{ ...graph.flows[0], target: graph.nodes[2].id }] }));
});


test('5,000 nodes retain positive geometry and a chosen fold preserves every flow', () => {
  const nodes = Array.from({ length: 5_000 }, (_, index) => node(`content ${index}`));
  const flows = Array.from({ length: 20_000 }, (_, index) => ({ id: `flow ${index}`,
    source: nodes[index < 13 ** 2 ? index % 13 : index % nodes.length].id,
    target: nodes[index < 13 ** 2 ? Math.floor(index / 13) : (index * 13 + 17) % nodes.length].id, value: index % 101 + 1 }));
  const full = new ChordMotion(); full.set({ nodes, flows });
  const state = full.advance(1, 0);
  const layout = layoutChord(state.data, full.presence, state.ringPresence);
  assert.equal(layout.arcs.length, nodes.length);
  assert.ok(layout.arcs.every(arc => arc.end > arc.start));
  assert.ok(layout.ribbons.every(ribbon => ribbon.sourceEnd > ribbon.sourceStart && ribbon.targetEnd > ribbon.targetStart));
  near(layout.arcs.reduce((sum, arc) => sum + arc.end - arc.start + chordGap(nodes.length) * arc.presence, 0), 2 * Math.PI);
  const folded = foldChord({ nodes, flows }, new Set(nodes.slice(0, 12).map(node => node.id)), node('fold'));
  assert.equal([...folded.flows.values()].reduce((sum, members) => sum + members.length, 0), 20_000);
  assert.equal(folded.data.nodes.length, 13); assert.equal(folded.data.flows.length, 13 ** 2);
  const motion = new ChordMotion(); motion.set(folded.data);
  const foldedState = motion.advance(1, 0);
  assert.equal(layoutChord(foldedState.data, motion.presence, foldedState.ringPresence).ribbons.length, folded.data.flows.length);
});

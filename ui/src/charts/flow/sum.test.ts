import assert from 'node:assert/strict';
import test from 'node:test';
import { OTHER, sumFlowParts } from './sum.ts';

test('other nodes retain the exact parts and sum every omitted country', () => {
  const parts = ['A', 'B', 'C', 'D', 'E'].map((source, index) => ({ part: source, source, target: 'Target', value: 5 - index }));
  const diagram = sumFlowParts(parts, 2);
  const other = diagram.nodes.find(node => node.side === 'source' && node.tag == null)!;
  assert.deepEqual(other.members, ['C', 'D', 'E']);
  assert.equal(other.value, 6);
  assert.equal(diagram.total, 15);
  assert.deepEqual(diagram.links.find(link => link.source === other.id)!.parts, ['C', 'D', 'E']);
  assert.equal(sumFlowParts(parts.slice(0, 3), 2).nodes.some(node => node.tag == null), false);
});

test('opaque tags and grouping keys cannot collide with summed nodes or ribbons', () => {
  const parts = [OTHER, 'A', 'A>t:B', 'C', 'D'].map((source, index) => ({ part: source, source, target: 'B>t:C', value: 10 - index }));
  const diagram = sumFlowParts(parts, 2);
  const sources = diagram.nodes.filter(node => node.side === 'source');
  assert.equal(new Set(sources.map(node => node.id)).size, sources.length);
  assert.ok(sources.some(node => node.tag === OTHER));
  assert.ok(sources.some(node => node.tag == null));
  const groups = sumFlowParts([{ part: 1, source: 'A', target: 'B', value: 1, group: 1 },
    { part: 2, source: 'A', target: 'B', value: 1, group: '1' }], 2);
  assert.equal(groups.links.length, 2);
});

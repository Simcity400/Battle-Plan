import assert from 'node:assert/strict';
import test from 'node:test';
import { countryFlowColours, countryFlowNodes, flowOtherColour } from './countries.ts';
import type { SummedNode } from './sum';

test('country flow nodes preserve recorded map colours and flagged unknown places', () => {
  const nodes: SummedNode[] = [{ id: 'a', side: 'source', tag: 'Country', members: ['Country'], memberValues: new Map(), value: 1 },
    { id: 'b', side: 'target', tag: 'Unknown', members: ['Unknown'], memberValues: new Map(), value: 1 },
    { id: 'other', side: 'source', tag: null, members: ['X', 'Y'], memberValues: new Map(), value: 1 }];
  const tokens = { surface: '#111111', text: '#dddddd', countryColours: 'raw' as const };
  const result = countryFlowNodes(nodes, tag => ({ flag: tag === 'Country' ? 'recorded flag' : undefined, flagReason: tag === 'Unknown' ? 'Native sprite refusal' : undefined }), tokens, node => node.tag ?? '2 other countries', node => node.tag!,
    countryFlowColours(['Country', 'Unknown'], tag => tag === 'Country' ? [45, 67, 89] : null, tokens.surface, false));
  assert.equal(result.nodes[0].color, '#2d4359');
  assert.equal(result.nodes[0].flag, 'recorded flag');
  assert.equal(result.nodes[1].color, flowOtherColour(tokens));
  assert.equal(result.nodes[1].flag, null);
  assert.equal(result.nodes[2].flag, undefined);
  assert.equal(result.nodes[2].last, true);
  assert.equal(result.nodes[1].countryFlag?.flagReason, 'Native sprite refusal');
  assert.equal(result.nodes[2].countryFlag, undefined);
});


test('opposite-side actors reuse one complete fixed colour selection across fitting and node order', () => {
  const tokens = { surface: '#111111', text: '#dddddd' };
  const colours = countryFlowColours(['A', 'B', 'Hidden', 'Unknown', 'A'], tag => tag === 'Unknown' ? null : [45, 67, 89], tokens.surface, true);
  const node = (tag: string, side: 'source' | 'target'): SummedNode => ({ id: `${side}:${tag}`, side, tag, members: [tag], memberValues: new Map(), value: 1 });
  const nodes = [node('A', 'source'), node('B', 'target'), node('B', 'source'), node('A', 'target'), node('Unknown', 'target')];
  const build = (selected: SummedNode[]) => countryFlowNodes(selected, () => ({}), tokens, node => node.tag!, node => node.tag!, colours);
  const full = build(nodes).nodes;
  assert.equal(full[0].color, full[3].color, 'A is identical on both sides');
  assert.equal(full[1].color, full[2].color, 'B is identical on both sides');
  assert.notEqual(full[0].color, full[1].color, 'the shared correction still distinguishes equal native colours');
  assert.equal(full[4].color, flowOtherColour(tokens), 'an unavailable native colour stays neutral');
  const fitted = build([nodes[3], nodes[0]]).nodes;
  assert.deepEqual(fitted.map(node => node.color), [full[0].color, full[0].color]);
  const reordered = build([...nodes].reverse()).nodes;
  assert.deepEqual(new Map(reordered.map(node => [node.id, node.color])), new Map(full.map(node => [node.id, node.color])));
  assert.deepEqual(countryFlowColours(['Hidden', 'B', 'A', 'Unknown'], tag => tag === 'Unknown' ? null : [45, 67, 89], tokens.surface, true), colours);
});

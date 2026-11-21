import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { SunburstNode } from './layout';
import { pickSunburst } from './pick.ts';

const node = { id: 'slice-401', key: '["entity","401"]', label: 'Drawn name', value: 10 } as SunburstNode;

test('slice activation retains the actual drawn native node, callback and held hour through head/name/playback refresh', () => {
  const owner = { source: 'recording-A', branch: '1', view: 'share', numbers: {} };
  const calls: unknown[] = [];
  const frame = { owner, nodes: [node], onPick: (selected: SunburstNode) => { calls.push([140, selected.key, selected.label]); return true; } };
  const refreshed = { owner, nodes: [{ ...node, label: 'New content name' }],
    onPick: (selected: SunburstNode) => { calls.push([141, selected.key, selected.label]); return true; } };
  assert.equal(pickSunburst(frame, refreshed.owner, true, node.id), true);
  assert.deepEqual(calls, [[140, '["entity","401"]', 'Drawn name']]);
  assert.equal(pickSunburst(refreshed, owner, true, node.id), true);
  assert.equal(calls.length, 2);
});

test('source/branch/view/numeric changes, missing slices and inactive charts cannot activate older geometry', () => {
  const owner = { source: 'A', branch: '1', view: 'share', numbers: {} };
  let called = false;
  const frame = { owner, nodes: [node], onPick: () => { called = true; return true; } };
  for (const next of [{ ...owner, source: 'B' }, { ...owner, branch: '2' }, { ...owner, view: 'race' },
    { ...owner, numbers: {} }, { ...owner }]) assert.equal(pickSunburst(frame, next, true, node.id), false);
  assert.equal(pickSunburst(frame, owner, false, node.id), false);
  assert.equal(pickSunburst(frame, owner, true, 'missing-native-id'), false);
  assert.equal(called, false);
});

test('default or declined activation retains zoom/Jump behavior, including an unpickable remainder', () => {
  const owner = {};
  assert.equal(pickSunburst({ owner, nodes: [node] }, owner, true, node.id), false);
  assert.equal(pickSunburst({ owner, nodes: [node], onPick: () => false }, owner, true, node.id), false);
  const rest = { ...node, id: 'rest', key: '["rest-of-world"]' };
  let called = false;
  assert.equal(pickSunburst({ owner, nodes: [rest], canPick: () => false,
    onPick: () => { called = true; return true; } }, owner, true, rest.id), false);
  assert.equal(called, false);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeTransfers } from './territoryData.ts';
import { linkEnds, selectedTransfers, transferSelection, transferSelectionToken } from './transferDrillData.ts';
import { deal, holder, reply } from './transfers.fixture.ts';
import type { Move } from './transfers.fixture.ts';

const move = (state: number, from: Move['from'], to: Move['to'], after: number): Move => ({ state, from, to, before: after - 1, after });
const a = holder(1, 'AAA', 100), b = holder(2, 'BBB', 100), c = holder(3, 'CCC', 100);
const current = reply([deal('one', [move(1, a, b, 101), move(2, a, c, 101), move(3, c, b, 101)])]);
const data = decodeTransfers(current);

test('a pinned country keeps its native holder; a pin from the flow diagram keeps its links', () => {
  const pinned = transferSelectionToken('campaign', 'branch', [], 5, '[2,1002]');
  assert.deepEqual(transferSelection(pinned, 'campaign', 'branch'), { ids: [], fromOffset: 5, country: '[2,1002]' });
  assert.equal(transferSelection(pinned, 'campaign', 'other'), null);
  assert.equal(transferSelection(pinned, 'other', 'branch'), null);
  const link = current.transfers.links.find(item => item.source === 'source:1:1001' && item.target === 'target:2:1002')!.id;
  const old = transferSelectionToken('campaign', 'branch', [link]);
  assert.deepEqual(transferSelection(old, 'campaign', 'branch'), { ids: [link], fromOffset: null, country: null });
  assert.deepEqual(transferSelection('', 'campaign', 'branch'), { ids: [], fromOffset: null, country: null });
});

test('selections pick their transfers and count saved links this scope lacks', () => {
  const country = selectedTransfers(data.all, { ids: [], fromOffset: null, country: '[2,1002]' });
  assert.deepEqual(country.shown.map(item => item.state), [1, 3]);
  const link = current.transfers.links.find(item => item.source === 'source:1:1001' && item.target === 'target:2:1002')!.id;
  assert.deepEqual(linkEnds(link), ['[1,1001]', '[2,1002]']);
  const links = selectedTransfers(data.all, { ids: [link, '20:source:9:1009target:8:1008', 'not a link'], fromOffset: null, country: null });
  assert.deepEqual(links.shown.map(item => item.state), [1]);
  assert.equal(links.missing, 2);
  assert.equal(selectedTransfers(data.all, { ids: [], fromOffset: null, country: null }).shown.length, 3);
});


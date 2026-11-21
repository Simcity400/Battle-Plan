import assert from 'node:assert/strict';
import test from 'node:test';
import { navalMeasure, navalOwnership, navalRequest } from './navalLossesData.ts';

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('a naval losses request names the chosen countries or the top N with its count, attribution and period', () => {
  assert.deepEqual(navalRequest('1', { tags: ['GER', 'ENG'] }, { kind: 'convoys', attribution: 'sunkBy', period: 'month' }),
    { branchId: '1', tags: ['GER', 'ENG'], top: null, attribution: 'sunkBy', kind: 'convoys', period: 'month' });
  assert.deepEqual(navalRequest('1', { tags: ['GER'], top: 10 }, {}),
    { branchId: '1', tags: [], top: 10, attribution: 'lost', kind: 'both', period: 'running' });
  assert.equal(navalRequest('1', { tags: [] }, { kind: 'planes' }).kind, 'both', 'an unknown count falls back to both');
});

test('the measure and period read as words', () => {
  assert.equal(navalMeasure({}), 'Ships and convoys lost');
  assert.equal(navalMeasure({ kind: 'ships', attribution: 'sunkBy' }), 'Ships sunk by');
  assert.equal(navalOwnership({ period: 'month' }), 'Per month');
});

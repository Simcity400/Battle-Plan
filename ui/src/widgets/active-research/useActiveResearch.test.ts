import assert from 'node:assert/strict';
import test from 'node:test';
import { activeResearchReply } from './useActiveResearch.ts';

const data = { history: { coverage: [{ firstHour: 100, lastHour: 105 }], slots: [{ index: 0,
  availability: [{ firstHour: 100, lastHour: 105 }], entries: [],
  states: [{ firstHour: 100, lastHour: 105, token: 'mod/technology', entryFirstObservedHour: null,
    speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0' }],
  progress: [{ firstHour: 100, lastHour: 100, entryFirstObservedHour: null, baseDaysRaw: '10000000', progressRaw: '5000000' }] }] },
  artwork: [], folders: [], technologyFolders: [], warnings: [] };

test('active research consumes the exact hour state while attribution is pending', () => {
  const first = activeResearchReply({ states: [{ tag: 'MOD', hour: 105, data, grants: { nextId: 0, grants: [], unavailable: null }, preparing: ['assignment', 'breakdown', 'remainingDays'] }] }, 'MOD', 105);
  assert.equal(first.historyPreparing, true);
  assert.equal(first.countdownPreparing, true);
  assert.deepEqual(first.data.history, data.history);
  const exact = activeResearchReply({ states: [{ tag: 'MOD', hour: 105, data, grants: { nextId: 0, grants: [], unavailable: null }, preparing: [] }] }, 'MOD', 105);
  assert.equal(exact.historyPreparing, false);
  assert.deepEqual(first.data, exact.data, 'exact slot values do not change with preparation');
  assert.throws(() => activeResearchReply({ states: [{ tag: 'MOD', hour: 106, data, preparing: [] }] }, 'MOD', 105), /Invalid active research/);
});

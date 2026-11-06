import assert from 'node:assert/strict';
import test from 'node:test';
import { lostAt } from '../charts/gaps.ts';
import { forgetReplyGaps, gapProduct, noteReplyGaps, replyGaps } from './replyGaps.ts';

const hole = (firstHour: number, lastHour: number, gap: boolean, family = 'equipment_losses') =>
  ({ firstHour, lastHour, family, unavailable: 'lost', reason: 'Synthetic damage', gap });

test('replies keep the lost spans their families went on past, by product and branch', () => {
  noteReplyGaps('gaps', 'equipment-losses', { branchId: '1' }, [hole(30, 39, true), hole(80, 99, false)]);
  noteReplyGaps('gaps', 'series-next-change', { branchId: '1', family: { kind: 'equipment-losses' } }, [hole(10, 19, true), hole(40, 44, true)]);
  noteReplyGaps('gaps', 'naval-losses', { selection: { branchId: '1' } }, [hole(60, 61, true, 'naval')]);
  noteReplyGaps('gaps', 'equipment-losses', { branchId: '2' }, [hole(5, 6, true)]);
  noteReplyGaps('gaps', 'equipment-losses', {}, [hole(1, 2, true)]);
  noteReplyGaps('gaps', 'series-summary', { branchId: '1' }, [hole(1, 2, true)]);
  assert.deepEqual(replyGaps('gaps', '1', ['equipment-losses']), [{ firstHour: 10, lastHour: 19 }, { firstHour: 30, lastHour: 44 }]);
  // Another product's chart is never cut where only equipment losses lost hours.
  assert.deepEqual(replyGaps('gaps', '1', ['army-size']), []);
  assert.deepEqual(replyGaps('gaps', '1', ['naval-losses']), [{ firstHour: 60, lastHour: 61 }]);
  // A layer drawing both takes their union.
  assert.deepEqual(replyGaps('gaps', '1', ['naval-losses', 'equipment-losses']),
    [{ firstHour: 10, lastHour: 19 }, { firstHour: 30, lastHour: 44 }, { firstHour: 60, lastHour: 61 }]);
  assert.deepEqual(replyGaps('gaps', '2', ['equipment-losses']), [{ firstHour: 5, lastHour: 6 }]);
  assert.equal(lostAt(replyGaps('gaps', '1', ['equipment-losses']), 44)?.firstHour, 30);
  assert.equal(lostAt(replyGaps('gaps', '1', ['equipment-losses']), 45), null);
  forgetReplyGaps('gaps');
  assert.deepEqual(replyGaps('gaps', '1', ['equipment-losses']), []);
});

test('a series request replies for its family kind', () => {
  assert.equal(gapProduct('series-summary', { family: { kind: 'naval-losses' } }), 'naval-losses');
  assert.equal(gapProduct('series-state', {}), null);
  assert.equal(gapProduct('casualty-pairs', { family: { kind: 'other' } }), 'casualty-pairs');
});

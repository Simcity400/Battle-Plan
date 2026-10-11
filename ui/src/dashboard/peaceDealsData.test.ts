import test from 'node:test';
import assert from 'node:assert/strict';
import { peaceCaptureLabel } from './peaceDealsData.ts';

test('unrecorded peace outcomes use one plain status at the actual failure date', () => {
  const deal = { captureUnavailable: 'Synthetic native operand refusal', captureUnavailableHour: 50 };
  assert.equal(peaceCaptureLabel(deal, 49), null);
  assert.equal(peaceCaptureLabel(deal, 50), 'Outcomes not recorded');
  assert.equal(peaceCaptureLabel(deal), 'Outcomes not recorded');
  assert.equal(peaceCaptureLabel({ captureUnavailable: null }), null);
});

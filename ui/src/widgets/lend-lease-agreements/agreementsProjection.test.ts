import assert from 'node:assert/strict';
import test from 'node:test';
import { lastDelivery } from './agreementsProjection.ts';

const item = (amountRaw: string) => ({ variant: '6', name: null, amountRaw });

test('a last delivery is a date when recorded, no delivery only when nothing was delivered, and otherwise not recorded', () => {
  assert.deepEqual(lastDelivery({ lastDelivery: 60_800_000, delivered: [item('500000')] }), lastDelivery({ lastDelivery: 60_800_000, delivered: [] }));
  assert.equal(lastDelivery({ lastDelivery: 60_800_000, delivered: [] }).kind, 'known');
  assert.equal(lastDelivery({ lastDelivery: 0, delivered: [item('0')] }).kind, 'notApplicable');
  assert.equal(lastDelivery({ lastDelivery: 0, delivered: [item('500000')] }).kind, 'notRecorded');
});

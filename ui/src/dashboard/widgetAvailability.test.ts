import assert from 'node:assert/strict';
import test from 'node:test';
import { availabilityReason, validateWidgetAvailability } from './widgetAvailability.ts';

test('feature entries require evidence, while ordinary widgets stay offered', () => {
  assert.equal(availabilityReason(false, false), null);
  assert.match(availabilityReason(true, false)!, /Load a campaign/);
  assert.match(availabilityReason(true, true)!, /Checking/);
  assert.equal(availabilityReason(true, true, { available: false, unavailable: null }), 'Not in this recording');
  assert.equal(availabilityReason(true, true, { available: null, unavailable: 'Feature mask not recorded' }), 'Feature mask not recorded');
  assert.equal(availabilityReason(true, true, { available: true, unavailable: null }), null);
  assert.match(availabilityReason(true, true, undefined, 'No content folders')!, /No content folders/);
});

test('availability never treats missing as false or an empty success', () => {
  assert.throws(() => validateWidgetAvailability({}), /malformed/);
  assert.throws(() => validateWidgetAvailability({ available: 0, unavailable: null }), /malformed/);
  assert.equal(validateWidgetAvailability({ available: null, unavailable: null }).available, null);
});

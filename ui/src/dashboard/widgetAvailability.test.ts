import assert from 'node:assert/strict';
import test from 'node:test';
import { availabilityReason, scopeReason, validateWidgetAvailability } from './widgetAvailability.ts';
import { definition as map } from '../widgets/map/definition.ts';
import { definition as wars } from '../widgets/wars/definition.ts';

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
  assert.throws(() => validateWidgetAvailability({ available: true, unavailable: 3 }), /malformed/);
});

test('a Map recording holds only the widgets that declare its mode', () => {
  assert.equal(scopeReason(wars, 'full'), null);
  assert.match(scopeReason(wars, 'map')!, /^Not in this recording: a Map recording holds the campaign map and its battle windows only\.$/);
  assert.equal(scopeReason(map, 'map'), null);
  assert.equal(scopeReason(map, 'full'), null);
  // No recording yet, or a widget that reads none, decides nothing here.
  assert.equal(scopeReason(wars, undefined), null);
  assert.equal(scopeReason({ data: false }, 'map'), null);
});

test('a reply with no reason field is read by its availability alone', () => {
  assert.deepEqual(validateWidgetAvailability({ available: true, firstHour: 1, lastHour: 2 }), { available: true, unavailable: null });
  assert.equal(availabilityReason(true, true, validateWidgetAvailability({ available: false })), 'Not in this recording');
});

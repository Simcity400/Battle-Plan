import assert from 'node:assert/strict';
import test from 'node:test';
import { RefusedError, ShownError } from './queryError.ts';
import { availabilityState, validateWidgetAvailability } from './widgetAvailability.ts';

test('validateWidgetAvailability accepts tail and availability replies and rejects malformed shapes', () => {
  assert.deepEqual(
    validateWidgetAvailability({ branchId: '1', total: '0', offset: '0', rows: [], missing: [], nextChange: null, unavailable: null, warnings: [] }),
    { available: true, unavailable: null },
  );
  assert.deepEqual(
    validateWidgetAvailability({ branchId: '1', unavailable: 'Construction was not recorded.' }),
    { available: false, unavailable: 'Construction was not recorded.' },
  );
  assert.deepEqual(
    validateWidgetAvailability({ available: true, unavailable: null }),
    { available: true, unavailable: null },
  );
  assert.deepEqual(
    validateWidgetAvailability({ available: null, unavailable: null }),
    { available: null, unavailable: null },
  );
  assert.deepEqual(
    validateWidgetAvailability({ available: false, unavailable: 'The content declares no raid type.' }),
    { available: false, unavailable: 'The content declares no raid type.' },
  );

  for (const malformed of [
    null,
    undefined,
    [],
    'ok',
    42,
    {},
    { branchId: '1', rows: [] },
    { unavailable: undefined },
    { unavailable: '' },
    { unavailable: '   ' },
    { unavailable: 123 },
    { available: true, unavailable: 'Contradiction.' },
    { available: false, unavailable: null },
    { available: 'yes', unavailable: null },
  ]) {
    assert.throws(() => validateWidgetAvailability(malformed), /malformed/i);
  }
});

test('availabilityState separates recorded, not recorded with reason, pending, and failed checks', () => {
  assert.deepEqual(availabilityState(false, false), { kind: 'available' });
  assert.deepEqual(availabilityState(true, false), { kind: 'pending', text: 'Load a campaign to check availability.' });
  assert.deepEqual(availabilityState(true, true), { kind: 'pending', text: 'Checking this recording…' });

  const recorded = validateWidgetAvailability({ unavailable: null });
  assert.deepEqual(availabilityState(true, true, recorded), { kind: 'available' });

  const unrecorded = validateWidgetAvailability({ unavailable: 'Construction was not recorded.' });
  assert.deepEqual(availabilityState(true, true, unrecorded), {
    kind: 'unavailable',
    reason: 'Construction was not recorded.',
  });

  const unknown = validateWidgetAvailability({ available: null, unavailable: null });
  assert.deepEqual(availabilityState(true, true, unknown), {
    kind: 'unavailable',
    reason: 'Availability not known',
  });

  const refused = availabilityState(true, true, undefined, new RefusedError('This recording predates espionage operations.'));
  assert.deepEqual(refused, {
    kind: 'unavailable',
    reason: 'This recording predates espionage operations.',
    detail: 'This recording predates espionage operations.',
  });

  let malformedError: Error | undefined;
  try {
    validateWidgetAvailability({ branchId: '1', rows: [] });
  } catch (error) {
    malformedError = error as Error;
  }
  assert.ok(malformedError);
  const failed = availabilityState(true, true, undefined, malformedError);
  assert.equal(failed.kind, 'error');
  assert.equal(failed.message, 'Availability check failed.');
  assert.equal(failed.detail, malformedError.message);
  assert.doesNotMatch(failed.message, /unavailable/i);

  const shown = availabilityState(true, true, undefined, new ShownError('Dashboard bridge disconnected.', 'pipe closed'));
  assert.deepEqual(shown, {
    kind: 'error',
    message: 'Dashboard bridge disconnected.',
    detail: 'pipe closed',
  });
});

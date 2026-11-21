import assert from 'node:assert/strict';
import test from 'node:test';
import { countryFlagUnavailable } from './countryFlagPresentation.ts';

test('missing, loading and decode-failed flags have concise labels and keep internal reasons', () => {
  assert.deepEqual(countryFlagUnavailable('Synthetic native refusal'), { reason: 'Synthetic native refusal', text: 'Flag unavailable' });
  assert.deepEqual(countryFlagUnavailable(), { reason: undefined, text: 'Flag unavailable' });
  assert.deepEqual(countryFlagUnavailable('Synthetic pending detail', true), { reason: 'Synthetic pending detail', text: 'Flag loading' });
  assert.deepEqual(countryFlagUnavailable(undefined, false, true), { reason: 'Couldn’t display the flag image.', text: 'Flag unavailable' });
});

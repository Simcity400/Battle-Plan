import assert from 'node:assert/strict';
import test from 'node:test';
import { RefusedError } from '../../dashboard/queryError.ts';
import { currentTooltip } from './researchLayout.ts';

test('technology artwork failures keep recorded titles and expose their own reason', () => {
  const refusal = new RefusedError('Synthetic technology artwork is unsupported');
  for (const failure of [refusal, new Error('Synthetic image decoder detail')]) {
    const tip = currentTooltip('synthetic', 0, null, null, false, {
      name: () => 'Recorded technology', icon: () => undefined, folder: () => undefined, artworkFailure: () => failure,
    });
    assert.equal(tip.title, 'Recorded technology');
    assert.deepEqual(tip.rows.at(-1), { label: failure === refusal ? refusal.text : 'Couldn’t load artwork.', detail: failure.message });
  }
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { EMPTY_VIEW } from '../view.ts';
import { labelMoment, viewForExtent } from './extent.ts';

test('grow with playback keeps a chosen zoom or pan while default source changes reset it', () => {
  const zoomed = { x: [20, 40] as const, y: null };
  assert.equal(viewForExtent(zoomed, '0:50', '0:60', true), zoomed);
  assert.deepEqual(viewForExtent(zoomed, '0:50', '0:60', true).x, [20, 40]);
  assert.equal(viewForExtent(zoomed, '0:50', '0:60'), EMPTY_VIEW);
  assert.equal(viewForExtent(zoomed, '0:50', '0:50'), zoomed);
  assert.equal(viewForExtent(zoomed, '', '0:50'), zoomed);
  assert.equal(viewForExtent(EMPTY_VIEW, '0:50', '0:60', true), EMPTY_VIEW);
});

test('an explicit label read hour avoids naming the right edge after the final recording hour', () => {
  assert.equal(labelMoment(null, 101, 100), 100);
  assert.equal(labelMoment(40, 101, 40), 40);
  assert.equal(labelMoment(null, 100), 100);
  assert.equal(labelMoment(40, 100), 40);
});

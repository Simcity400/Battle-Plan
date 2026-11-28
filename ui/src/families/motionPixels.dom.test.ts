import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('ordered bars settle to the same pixels as a cold draw after worker playback', async () => {
  await withNativePage('ordered-bars-pixel-settle', '/ui/src/families/motionPixels.dom.fixture.ts', async page => {
    const results = await page.evaluate<{ dpr: number; moving: number; paused: number }[]>('window.checkSettledPixels()');
    for (const result of results) {
      assert.ok(result.moving > 0, `no movement at DPR ${result.dpr}`);
      assert.equal(result.paused, 0, `paused picture differs at DPR ${result.dpr}`);
    }
  });
});

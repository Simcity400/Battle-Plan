import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('ordered bars settle to the same pixels as a cold draw after worker playback', async () => {
  await withNativePage('ordered-bars-pixel-settle', '/ui/src/families/motionPixels.dom.fixture.ts', async page => {
    const { results, histories, afterScroll, afterResize, afterTick, afterRefusal } = await page.evaluate<{
      results: { dpr: number; moving: number; paused: number }[];
      histories: { revoked: boolean; history: string; difference: number }[];
      afterScroll: number; afterResize: boolean; afterTick: number; afterRefusal: number;
    }>('window.checkSettledPixels()');
    for (const result of results) {
      assert.ok(result.moving > 0, `no movement at DPR ${result.dpr}`);
      assert.equal(result.paused, 0, `paused picture differs at DPR ${result.dpr}`);
    }
    for (const sample of histories) assert.equal(sample.difference, 0, `${sample.history} with endpoint refusal ${sample.revoked}`);
    assert.equal(afterScroll, 0, 'layout must not copy other pixels into the owned slot');
    assert.equal(afterResize, false, 'layout must not fill the owned slot after the backing canvas resizes');
    assert.equal(afterTick, 0, 'the tick restores the owned pixels at the same clock hour');
    assert.equal(afterRefusal, 0, 'a proven unavailable hour matches its recorded cold picture');
  });
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';
import type { ParityFailure } from './pixelParity.dom.fixture.ts';

test('time-chart cached draw matches full uncached draw pixel-for-pixel across playback, seek, zoom, resize, line enter/leave, gap, partial span, stack, and columns', async () => {
  await withNativePage('time-chart-pixel-parity', '/ui/src/families/time-chart/pixelParity.dom.fixture.ts', page => page.stage('pixel-parity', async () => {
    const result = await page.evaluate<{ readonly checked: number; readonly failures: ParityFailure[] }>('window.timeChartPixelParity()');
    assert.equal(result.checked, 19);
    assert.deepEqual(result.failures, [], `cached draw differed from full uncached draw: ${JSON.stringify(result.failures)}`);
  }));
});

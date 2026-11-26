import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

type Shown = { count: boolean; compact: boolean; label: string | null };

test('a narrow header shows a live time chip as its dot once the count is dropped, and refits at once when the time mode changes during playback', async () => {
  await withNativePage('widget-header', '/ui/src/dashboard/widgetHeader.dom.fixture.tsx', page => page.stage('widget header', async () => {
    assert.deepEqual(await page.evaluate<Shown>('window.widgetHeader.mount()'), { count: false, compact: true, label: null },
      'the count does not fit, so it is dropped and the live chip shows only its dot');
    assert.deepEqual(await page.evaluate<Shown>(`window.widgetHeader.switchWhilePlaying({ mode: 'campaign' })`), { count: false, compact: false, label: 'Whole campaign' },
      'switching to the whole campaign while playing shows its label and keeps the count dropped');
    assert.deepEqual(await page.evaluate<Shown>(`window.widgetHeader.switchWhilePlaying({ mode: 'follow' })`), { count: false, compact: true, label: null },
      'switching back to live while playing shows only the dot again');
  }, 60_000));
});

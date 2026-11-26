import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

type Shown = { count: boolean; compact: boolean; label: string | null };

test('a header whose count fits only without the time chip shows a live chip as its dot, and refits at once on every time-mode change during playback', async () => {
  await withNativePage('widget-header', '/ui/src/dashboard/widgetHeader.dom.fixture.tsx', page => page.stage('widget header', async () => {
    const live: Shown = { count: false, compact: true, label: null };
    assert.deepEqual(await page.evaluate<Shown>('window.widgetHeader.mount()'), live, 'the count is dropped and the live chip shows only its dot');
    for (const [at, mode] of (['campaign', 'follow', 'campaign'] as const).entries()) {
      const shown = await page.evaluate<Shown>(`window.widgetHeader.switchWhilePlaying({ mode: '${mode}' })`);
      assert.equal(shown.count, false, `switch ${at + 1} to ${mode} while playing refits at once: the count stays dropped (${JSON.stringify(shown)})`);
      if (mode === 'follow') assert.deepEqual(shown, live, `switch ${at + 1}: back on live, only the dot`);
      else assert.deepEqual(shown, { count: false, compact: false, label: 'Whole campaign' }, `switch ${at + 1}: the whole campaign's chip at full width`);
    }
  }, 60_000));
});

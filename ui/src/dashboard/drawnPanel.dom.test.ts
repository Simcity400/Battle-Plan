import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('a drawn panel takes every field from the installed texts: a bar at half fills half, and a key the picture lacks reads Not recorded', async () => {
  await withNativePage('drawn-panel', '/ui/src/dashboard/drawnPanel.dom.fixture.tsx', page => page.stage('drawn panel', async () => {
    const shown = await page.evaluate<{ fill: number; progress: string; iterations: string }>(
      `window.drawnPanel.project(0.2, [['fast:progress', '50%'], ['fast:bar', '0.5']])`);
    assert.deepEqual(shown, { fill: 0.5, progress: '50%', iterations: 'Not recorded' });
  }));
});

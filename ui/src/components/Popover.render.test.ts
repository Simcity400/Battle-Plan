import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

test('a popover opens below its anchor, and one that cannot opens beside the column it must not cover', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { popoverPlacement } = await vite.ssrLoadModule('/ui/src/components/Popover.tsx') as typeof import('./Popover.tsx');
    const viewport = { width: 1600, height: 900 };
    const addRow = { left: 40, right: 40, top: 300, bottom: 300, aside: 220 };
    assert.deepEqual(popoverPlacement(addRow, 340, 420, viewport, 'start'), { left: 40, top: 306 }, 'room below: under the Add region row, aligned with the names');
    const low = { ...addRow, top: 700, bottom: 700 };
    assert.deepEqual(popoverPlacement(low, 340, 420, viewport, 'start'), { left: 226, top: 468 }, 'no room below: beside the name column, never over it');
    assert.deepEqual(popoverPlacement({ left: 40, right: 40, top: 700, bottom: 700 }, 340, 420, viewport, 'start'), { left: 40, top: 274 }, 'without a column to keep clear it flips above as before');
  } finally {
    await vite.close();
  }
});

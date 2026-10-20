import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient } from '../dashboard/client';

test('the renderer registers the policy chooser and opens the organisation\'s policy screen at the selection\'s hour', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const [seam, registry] = await Promise.all([vite.ssrLoadModule('/ui/src/dashboard/gameWindows.ts'),
      vite.ssrLoadModule('/ui/src/components/gameWindowRegistry.tsx')]) as [typeof import('../dashboard/gameWindows.ts'),
      typeof import('./gameWindowRegistry.tsx')];
    assert.equal(seam.gameWindow('mioPolicyChooser'), registry.MioPolicyChooser);
    const client = {} as DashboardClient;
    const selection = { kind: 'mioPolicyChooser' as const, sourceId: 'synthetic', revision: '2', branchId: '7', hour: 120,
      organisation: '18446744073709551615' };
    // The window's own diagnostics reach the widget that opened it.
    const onDiagnostics = (lines: string[]) => { void lines; };
    assert.deepEqual(registry.mioPolicyWindow({ selection, active: false, client, onDiagnostics }), {
      client, source: { sourceId: 'synthetic', revision: '2' }, branchId: '7', hour: 120,
      target: { kind: 'mioPolicies', organisation: '18446744073709551615' }, active: false, onDiagnostics,
    });
  } finally {
    await vite.close();
  }
});

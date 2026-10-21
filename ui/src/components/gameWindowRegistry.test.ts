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

test('the renderer registers the division designer and opens the template designer at the row hour', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const [seam, registry] = await Promise.all([vite.ssrLoadModule('/ui/src/widgets/division-template/designerSnapshot.ts'),
      vite.ssrLoadModule('/ui/src/components/gameWindowRegistry.tsx')]) as [typeof import('../widgets/division-template/designerSnapshot.ts'),
      typeof import('./gameWindowRegistry.tsx')];
    assert.notEqual(seam.designerSnapshot(), null);
    const client = {} as DashboardClient;
    const onDiagnostics = (lines: string[]) => { void lines; };
    const request = { client, source: { id: 'synthetic', path: 'synthetic.battleplan', revision: '3', metadata: {} } as never, branchId: '7',
      hour: 240, tag: 'GER', template: '18446744073709551615', designer: 'ordinary' as const, onDiagnostics };
    // The window is named by the template's full handle alone; the tag and kind are the widget's.
    assert.deepEqual(registry.designerWindow(request, false), {
      client, source: { sourceId: 'synthetic', revision: '3' }, branchId: '7', hour: 240,
      target: { kind: 'divisionDesigner', template: '18446744073709551615' }, active: false, onDiagnostics,
    });
  } finally {
    await vite.close();
  }
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

test('native query rejections use typed failures and cancellation keeps AbortError', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'synthetic-native-ipc', enforce: 'pre',
      resolveId(id) { if (id === '@tauri-apps/api/core') return '\0synthetic-native'; if (id === '@tauri-apps/api/event') return '\0synthetic-events'; },
      load(id) { if (id === '\0synthetic-events') return 'export async function listen() { return () => {}; }'; if (id === '\0synthetic-native') return `export const isTauri = () => true;
        export async function invoke(command, args) { if (command === 'dashboard_cancel') return;
          throw globalThis.__typedQueryFailure; }`; },
    }] });
  const host = globalThis as typeof globalThis & { __typedQueryFailure?: unknown };
  try {
    const { nativeDashboardClient: client } = await vite.ssrLoadModule('/ui/src/dashboard/nativeClient.ts');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const query = { sourceId: 'synthetic', revision: '1', product: 'land-owners', params: {} };
    for (const failure of [{ kind: 'refused', text: 'Land owners are not prepared natively yet' }, { kind: 'failed', message: 'Synthetic technical detail' }]) {
      host.__typedQueryFailure = failure;
      await assert.rejects(client.product(query, new AbortController().signal), (error: unknown) => {
        assert.ok(error instanceof Error);
        assert.equal(error instanceof RefusedError, failure.kind === 'refused');
        assert.equal(error.message, failure.kind === 'refused' ? failure.text : failure.message);
        return true;
      });
    }
    const cancelled = new AbortController();
    const read = client.product(query, cancelled.signal);
    cancelled.abort();
    await assert.rejects(read, { name: 'AbortError' });
  } finally { delete host.__typedQueryFailure; await vite.close(); }
});

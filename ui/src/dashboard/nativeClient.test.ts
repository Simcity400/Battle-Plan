import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { productRequest } from './request.ts';

test('native query rejections use typed failures and cancellation keeps AbortError', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, ssr: { noExternal: ['@tauri-apps/api'] },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'synthetic-native-ipc', enforce: 'pre',
      resolveId(id) { if (id === '@tauri-apps/api/core') return '\0synthetic-native'; if (id === '@tauri-apps/api/event') return '\0synthetic-events'; },
      load(id) { if (id === '\0synthetic-events') return 'export async function listen() { return () => {}; }'; if (id === '\0synthetic-native') return `export const isTauri = () => true;
        export async function invoke(command, args) { if (command === 'dashboard_cancel') return;
          if (command === 'set_dashboard_maps') { globalThis.__sentMapViews.push(args); return; }
          if (globalThis.__lateReply) { await globalThis.__lateReply; return new TextEncoder().encode('{"sourceId":"synthetic","revision":"1","product":"land-owners","data":{}}'); }
          throw globalThis.__typedQueryFailure; }`; },
    }] });
  const host = globalThis as typeof globalThis & { __typedQueryFailure?: unknown; __lateReply?: Promise<void>; __sentMapViews?: unknown[] };
  host.__sentMapViews = [];
  try {
    const { nativeDashboardClient: client } = await vite.ssrLoadModule('/ui/src/dashboard/nativeClient.ts');
    await client.setMapViews('page-B', []);
    assert.deepEqual(host.__sentMapViews, [{ clockId: 'page-B', views: [] }]);
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const query = productRequest({ sourceId: 'synthetic', revision: '1', product: 'land-owners', params: {} });
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
    let land!: () => void;
    host.__lateReply = new Promise<void>(resolve => { land = resolve; });
    const late = new AbortController();
    const parse = JSON.parse;
    let parsed = 0;
    JSON.parse = ((...args: Parameters<typeof JSON.parse>) => { parsed++; return parse(...args); }) as typeof JSON.parse;
    try {
      const lateRead = client.product(query, late.signal);
      await new Promise(resolve => setTimeout(resolve, 0));
      late.abort();
      land();
      await assert.rejects(lateRead, { name: 'AbortError' });
    } finally { JSON.parse = parse; }
    assert.equal(parsed, 0, 'a reply landing after its request was cancelled is not parsed');
  } finally { delete host.__typedQueryFailure; delete host.__lateReply; delete host.__sentMapViews; await vite.close(); }
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

test('native query rejections use typed failures and cancellation keeps AbortError', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, ssr: { noExternal: ['@tauri-apps/api'] },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'synthetic-native-ipc', enforce: 'pre',
      resolveId(id) { if (id === '@tauri-apps/api/core') return '\0synthetic-native'; if (id === '@tauri-apps/api/event') return '\0synthetic-events'; },
      load(id) { if (id === '\0synthetic-events') return 'export async function listen() { return () => {}; }'; if (id === '\0synthetic-native') return `export const isTauri = () => true;
        export async function invoke(command, args) { if (command === 'dashboard_cancel') return;
          if (globalThis.__lateReply) { await globalThis.__lateReply; return new TextEncoder().encode('{"sourceId":"synthetic","revision":"1","product":"land-owners","data":{}}'); }
          throw globalThis.__typedQueryFailure; }`; },
    }] });
  const host = globalThis as typeof globalThis & { __typedQueryFailure?: unknown; __lateReply?: Promise<void> };
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
    // Run 17: 107 replies landing after their requests were cancelled were parsed anyway inside playing windows (289 ms of
    // the main thread): a reply nobody waits on any more is dropped as it lands.
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
  } finally { delete host.__typedQueryFailure; delete host.__lateReply; await vite.close(); }
});

test('every recording\'s follow store hears the pushed messages over the client\'s one channel', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, ssr: { noExternal: ['@tauri-apps/api'] },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'follow-channel-ipc', enforce: 'pre',
      resolveId(id) { if (id === '@tauri-apps/api/core') return '\0follow-channel'; if (id === '@tauri-apps/api/event') return '\0follow-channel-events'; },
      load(id) {
        if (id === '\0follow-channel-events') return 'export async function listen() { return () => {}; }';
        // The desktop holds the channel registered last, as `dashboard_follow_channel` does.
        if (id === '\0follow-channel') return `export const isTauri = () => true; export class Channel { onmessage = () => {}; }
          export async function invoke(command, args) { if (command === 'dashboard_follow_channel') { globalThis.__followChannel = args.channel; globalThis.__followRegistrations++; } }`;
      } }] });
  const host = globalThis as typeof globalThis & { __followChannel?: { onmessage: (bytes: ArrayBuffer) => void }; __followRegistrations?: number };
  host.__followRegistrations = 0;
  try {
    const { nativeDashboardClient: client } = await vite.ssrLoadModule('/ui/src/dashboard/nativeClient.ts') as typeof import('./nativeClient.ts');
    const heard: string[] = [];
    const stopFirst = await client.onFollowMessages!(bytes => heard.push(`first ${new Uint8Array(bytes)[0]}`));
    const stopSecond = await client.onFollowMessages!(bytes => heard.push(`second ${new Uint8Array(bytes)[0]}`));
    host.__followChannel!.onmessage(new Uint8Array([1]).buffer);
    assert.deepEqual(heard, ['first 1', 'second 1'], 'a second recording\'s store leaves the first one hearing');
    assert.equal(host.__followRegistrations, 1, 'one channel for the client');
    stopFirst();
    host.__followChannel!.onmessage(new Uint8Array([2]).buffer);
    assert.deepEqual(heard.slice(2), ['second 2'], 'a store that stopped hears nothing more; the other still hears');
    stopSecond();
    await client.onFollowMessages!(bytes => heard.push(`third ${new Uint8Array(bytes)[0]}`));
    host.__followChannel!.onmessage(new Uint8Array([3]).buffer);
    assert.deepEqual(heard.slice(3), ['third 3'], 'a store listening again hears on the same channel');
    assert.equal(host.__followRegistrations, 1);
  } finally { delete host.__followChannel; delete host.__followRegistrations; await vite.close(); }
});

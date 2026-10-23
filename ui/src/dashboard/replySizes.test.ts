import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

test('releasing a recording through the desktop client forgets the reply sizes learned of it', async () => {
  const globals = globalThis as typeof globalThis & { __replySizeInvoke?: (command: string) => Promise<unknown> };
  globals.__replySizeInvoke = async () => undefined;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'reply-size-native', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/shell/api.ts')) return 'export const desktopAvailable = true;';
      if (path.endsWith('/dashboard/nativeClient.ts')) return source.replace(
        "import { invoke } from '@tauri-apps/api/core';", 'const invoke = command => globalThis.__replySizeInvoke(command);');
    } }] });
  try {
    const { nativeDashboardClient: client } = await vite.ssrLoadModule('/ui/src/dashboard/nativeClient.ts') as typeof import('./nativeClient.ts');
    const sizes = await vite.ssrLoadModule('/ui/src/dashboard/replySizes.ts') as typeof import('./replySizes.ts');
    sizes.setReplySize(client, 'native-lease', 'colours', { fits: 4, refused: 8 });
    sizes.setReplySize(client, 'other-lease', 'colours', { fits: 2, refused: 4 });
    await client.releaseSource('native-lease');
    assert.equal(sizes.replySize(client, 'native-lease', 'colours'), undefined);
    assert.deepEqual(sizes.replySize(client, 'other-lease', 'colours'), { fits: 2, refused: 4 });
  } finally { delete globals.__replySizeInvoke; await vite.close(); }
});

test('releasing a recording through the browser client forgets the reply sizes learned of it', async () => {
  const previous = globalThis.fetch;
  globalThis.fetch = (async () => ({ ok: true, text: async () => '{}' })) as unknown as typeof fetch;
  try {
    const { browserDashboardClient: client } = await import('./browserClient.ts');
    const sizes = await import('./replySizes.ts');
    sizes.setReplySize(client, 'browser-lease', 'colours', { fits: 4, refused: 8 });
    await client.releaseSource('browser-lease');
    assert.equal(sizes.replySize(client, 'browser-lease', 'colours'), undefined);
  } finally { globalThis.fetch = previous; }
});

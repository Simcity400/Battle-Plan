import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
const { showFlag } = await vite.ssrLoadModule('/ui/src/components/CountryFlag.tsx').finally(() => vite.close()) as typeof import('./CountryFlag.tsx');

test('a flag replacing another clears it at once, and draws only once its picture is decoded', async () => {
  const page = globalThis as unknown as Record<string, unknown>;
  const decodes: { resolve: () => void; reject: () => void }[] = [];
  const observed: unknown[] = [];
  Object.assign(page, {
    Image: class { src = ''; decode() { return new Promise<void>((resolve, reject) => decodes.push({ resolve, reject })); } },
    ResizeObserver: class { observe(target: unknown) { observed.push(target); } unobserve() {} },
  });
  try {
    let cleared = 0;
    const canvas = { width: 82, height: 52, getContext: () => ({ clearRect: () => { cleared++; } }) } as unknown as HTMLCanvasElement;
    const flag = (png: string) => ({ width: 82, height: 52, png });
    showFlag(canvas, flag('http://artwork.localhost/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.png'));
    assert.equal(cleared, 1);
    decodes[0].reject();
    await new Promise(resolve => setTimeout(resolve));
    assert.deepEqual(observed, []);
    const stop = showFlag(canvas, flag('http://artwork.localhost/cccccccccccccccccccccccccccccccc.png'));
    stop();
    showFlag(canvas, flag('http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png'));
    decodes[1].resolve();
    decodes[2].resolve();
    await new Promise(resolve => setTimeout(resolve));
    assert.equal(cleared, 3);
    assert.deepEqual(observed, [canvas]);
  } finally { for (const name of ['Image', 'ResizeObserver']) delete page[name]; }
});

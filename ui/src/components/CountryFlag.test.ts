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
    // A canvas still showing an earlier flag of the same size, which React leaves as it is.
    const canvas = { width: 82, height: 52, getContext: () => ({ clearRect: () => { cleared++; } }) } as unknown as HTMLCanvasElement;
    const flag = (png: string) => ({ width: 82, height: 52, png });
    // A replacement that cannot be decoded leaves the frame empty.
    showFlag(canvas, flag('data:image/png;base64,broken'));
    assert.equal(cleared, 1);
    decodes[0].reject();
    await new Promise(resolve => setTimeout(resolve));
    assert.deepEqual(observed, []);
    // One decoded is drawn; one stopped before its decode never is.
    const stop = showFlag(canvas, flag('data:image/png;base64,later'));
    stop();
    showFlag(canvas, flag('data:image/png;base64,QQ=='));
    decodes[1].resolve();
    decodes[2].resolve();
    await new Promise(resolve => setTimeout(resolve));
    assert.equal(cleared, 3);
    assert.deepEqual(observed, [canvas]);
  } finally { for (const name of ['Image', 'ResizeObserver']) delete page[name]; }
});

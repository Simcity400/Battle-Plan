import test from 'node:test';
import assert from 'node:assert/strict';
const freshDiscovery = (scenario: string) => import('./fontDiscovery.ts?' + scenario) as Promise<typeof import('./fontDiscovery.ts')>;

test('all font controls and later settings visits share discovery including denied access', async () => {
  const before = Object.getOwnPropertyDescriptor(globalThis, 'window');
  let calls = 0;
  try {
    Object.defineProperty(globalThis, 'window', { configurable: true, value: { queryLocalFonts: async () => {
      calls++; return [{ family: 'Synthetic B' }, { family: 'Synthetic A' }, { family: 'Synthetic A' }];
    } } });
    const granted = await freshDiscovery('granted');
    await Promise.all(Array.from({ length: 4 }, () => granted.discoverInstalledFonts()));
    await granted.discoverInstalledFonts();
    assert.equal(calls, 1);
    assert.deepEqual(granted.readFontDiscovery(), { status: 'granted', families: ['Synthetic A', 'Synthetic B'] });
    calls = 0;
    window.queryLocalFonts = async () => { calls++; throw new Error('Denied'); };
    const denied = await freshDiscovery('denied');
    await Promise.all([denied.discoverInstalledFonts(), denied.discoverInstalledFonts()]);
    await denied.discoverInstalledFonts();
    assert.equal(calls, 1);
    assert.deepEqual(denied.readFontDiscovery(), { status: 'unavailable' });
    window.queryLocalFonts = async () => [];
    const empty = await freshDiscovery('empty');
    await empty.discoverInstalledFonts();
    assert.deepEqual(empty.readFontDiscovery(), { status: 'granted', families: [] });
  } finally {
    if (before) Object.defineProperty(globalThis, 'window', before);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});

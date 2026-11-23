import assert from 'node:assert/strict';
import test from 'node:test';
import { FamilyArtLoader, type FamilyArtBitmap } from './artLoader.ts';

function fakeBitmap(closed: string[], id: string): FamilyArtBitmap {
  return { width: 20, height: 14, close() { closed.push(id); } };
}

test('F6: FamilyArtLoader bounds bitmap cache, closes evicted bitmaps, aborts scrolled-away fetches, and cleans up on dispose', async () => {
  const closed: string[] = [];
  const aborted: string[] = [];
  const resolvers = new Map<string, (bmp: FamilyArtBitmap) => void>();
  let loads = 0;

  const loader = new FamilyArtLoader<FamilyArtBitmap>({
    maxEntries: 2,
    onLoad: () => { loads++; },
    fetchBitmap: (url, signal) => new Promise<FamilyArtBitmap>((resolve, reject) => {
      signal.addEventListener('abort', () => {
        aborted.push(url);
        reject(new DOMException('Aborted', 'AbortError'));
      }, { once: true });
      resolvers.set(url, resolve);
    }),
  });

  loader.requestVisible(['a.png', 'b.png']);
  assert.equal(loader.pendingCount, 2);
  resolvers.get('a.png')!(fakeBitmap(closed, 'a.png'));
  resolvers.get('b.png')!(fakeBitmap(closed, 'b.png'));
  await Promise.resolve();
  assert.equal(loader.cachedCount, 2);
  assert.equal(loads, 2);

  loader.requestVisible(['b.png', 'c.png']);
  resolvers.get('c.png')!(fakeBitmap(closed, 'c.png'));
  await Promise.resolve();
  assert.equal(loader.cachedCount, 2);
  assert.deepEqual(closed, ['a.png'], 'oldest unreferenced bitmap closed on eviction');
  assert.equal(loader.get('a.png'), undefined);
  assert.ok(loader.get('b.png'));
  assert.ok(loader.get('c.png'));

  loader.requestVisible(['d.png']);
  assert.equal(loader.pendingCount, 1);
  loader.requestVisible(['e.png', 'f.png']);
  assert.deepEqual(aborted, ['d.png'], 'scrolled-away pending request is aborted');
  assert.equal(loader.pendingCount, 2);

  let lateResolve!: (bmp: FamilyArtBitmap) => void;
  const loader2 = new FamilyArtLoader<FamilyArtBitmap>({
    maxEntries: 2,
    fetchBitmap: () => new Promise<FamilyArtBitmap>(resolve => { lateResolve = resolve; }),
  });
  loader2.requestVisible(['late.png']);
  loader2.dispose();
  lateResolve(fakeBitmap(closed, 'late.png'));
  await Promise.resolve();
  assert.ok(closed.includes('late.png'), 'late-resolving bitmap after dispose is closed immediately');

  loader.dispose();
  assert.equal(loader.cachedCount, 0);
  assert.equal(loader.pendingCount, 0);
  assert.deepEqual(aborted, ['d.png', 'e.png', 'f.png'], 'dispose aborts remaining pending fetches');
  assert.deepEqual(closed.filter(id => id !== 'late.png').sort(), ['a.png', 'b.png', 'c.png'], 'dispose closes all cached bitmaps');
});

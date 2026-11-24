import assert from 'node:assert/strict';
import test from 'node:test';
import { DomArtCache, type DomArtImage } from './domCache.ts';
import { LaneArtCache } from './laneCache.ts';
import {
  CURRENT_INSTALLATION_ORIGIN,
  NO_ART,
  PENDING_ART,
  failedArt,
  missingArt,
  readyArt,
  refusedArt,
  type ArtBitmap,
} from './types.ts';

function fakeBitmap(): ArtBitmap & { closed: boolean } {
  return {
    width: 41,
    height: 26,
    closed: false,
    close() { this.closed = true; },
  };
}

test('LaneArtCache distinguishes all six states, wakes on load and failure, retries, and flushes on content revision', async () => {
  const closed: string[] = [];
  const aborted: string[] = [];
  let failOnce = true;
  let wakes = 0;

  const cache = new LaneArtCache<ArtBitmap>({
    maxEntries: 2,
    contentRevision: 1,
    onSettle: () => { wakes++; },
    fetchBitmap: (url, signal) => new Promise((resolve, reject) => {
      signal.addEventListener('abort', () => {
        aborted.push(url);
        reject(new DOMException('Aborted', 'AbortError'));
      }, { once: true });
      if (url === 'art://flaky' && failOnce) {
        failOnce = false;
        queueMicrotask(() => reject(new Error('network timeout')));
        return;
      }
      if (url === 'art://slow') return;
      queueMicrotask(() => {
        if (signal.aborted) return;
        const bmp = fakeBitmap();
        const orig = bmp.close.bind(bmp);
        bmp.close = () => { closed.push(url); orig(); };
        resolve(bmp);
      });
    }),
  });

  assert.deepEqual(cache.resolve(NO_ART), { kind: 'none' });
  assert.deepEqual(cache.resolve(PENDING_ART), { kind: 'pending' });
  assert.deepEqual(cache.resolve(missingArt('not in mod stack')), { kind: 'missing', reason: 'not in mod stack' });
  assert.deepEqual(cache.resolve(refusedArt('unsupported DDS format')), { kind: 'refused', reason: 'unsupported DDS format' });
  assert.equal(cache.resolve(failedArt('rpc failed')).kind, 'failed');

  const okRes = readyArt('art://ok', CURRENT_INSTALLATION_ORIGIN, { frame: { index: 2, count: 4 } });
  const flakyRes = readyArt('art://flaky', CURRENT_INSTALLATION_ORIGIN);
  const slowRes = readyArt('art://slow', CURRENT_INSTALLATION_ORIGIN);

  const flush = async () => { for (let i = 0; i < 4; i++) await Promise.resolve(); };

  cache.requestVisible([okRes, flakyRes, slowRes]);
  assert.equal(cache.resolve(okRes).kind, 'pending');
  assert.equal(cache.resolve(flakyRes).kind, 'pending');

  await flush();

  const ready = cache.resolve(okRes);
  assert.equal(ready.kind, 'ready');
  assert.equal(ready.origin.currentInstallation, true);
  assert.deepEqual(ready.frame, { index: 2, count: 4 });

  const failed = cache.resolve(flakyRes);
  assert.equal(failed.kind, 'failed');
  assert.equal(failed.reason, 'network timeout');
  assert.ok(wakes >= 2, 'wakes on both load and failure');

  failed.retry();
  await flush();
  assert.equal(cache.resolve(flakyRes).kind, 'ready', 'retry re-fetches and decodes');

  cache.requestVisible([okRes, flakyRes]);
  assert.deepEqual(aborted, ['art://slow'], 'scrolled-off pending fetch is aborted');

  cache.requestVisible([readyArt('art://third')]);
  await flush();
  assert.equal(cache.cachedCount, 2);
  assert.ok(closed.includes('art://ok'), 'evicted bitmap is closed');

  cache.setContentRevision(2);
  assert.equal(cache.cachedCount, 0);
  assert.equal(cache.resolve(flakyRes).kind, 'pending', 'content revision flush clears cache');

  cache.dispose();
});

test('DomArtCache distinguishes pending, ready, and failed, wakes listeners, retries, and flushes on content revision', () => {
  const created: (DomArtImage & { triggerLoad: () => void; triggerError: () => void })[] = [];
  const cache = new DomArtCache<DomArtImage>({
    maxEntries: 2,
    createImage: () => {
      const img = {
        complete: false,
        naturalWidth: 0,
        naturalHeight: 0,
        src: '',
        onload: null as (() => void) | null,
        onerror: null as (() => void) | null,
        triggerLoad() {
          this.complete = true;
          this.naturalWidth = 32;
          this.naturalHeight = 32;
          this.onload?.();
        },
        triggerError() {
          this.complete = true;
          this.naturalWidth = 0;
          this.naturalHeight = 0;
          this.onerror?.();
        },
      };
      created.push(img);
      return img;
    },
  });

  const res = readyArt('dom://icon1', CURRENT_INSTALLATION_ORIGIN);
  let wakes = 0;
  const unsub = cache.subscribe('dom://icon1', () => { wakes++; });

  assert.equal(cache.resolve(res).kind, 'pending');
  created[0]!.triggerError();
  assert.equal(wakes, 1, 'wakes subscriber on error');
  const failed = cache.resolve(res);
  assert.equal(failed.kind, 'failed');

  failed.retry();
  assert.equal(cache.resolve(res).kind, 'pending');
  created[1]!.triggerLoad();
  const ready = cache.resolve(res);
  assert.equal(ready.kind, 'ready');
  assert.equal(ready.width, 32);
  assert.equal(ready.origin.label, 'Current installation');

  cache.setContentRevision(2);
  assert.equal(cache.cachedCount, 0);
  unsub();
});

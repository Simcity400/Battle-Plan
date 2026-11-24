import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../boundedView.ts';
import { IdentityIndex } from '../identity/index.ts';
import { flagChoiceKey } from '../identity/types.ts';
import { DomArtCache, type DomArtImage } from './domCache.ts';
import { LaneArtCache } from './laneCache.ts';
import { drawMediaSlot, flagEntryToArt, flagImageToArt, resolveCachedArt, spriteImageToArt } from './adapters.ts';
import { artUrlForHandle, collectArtUrls, createDomArtProvider, createLaneArtProvider, flagArtHandle, laneArtDemand, urlArtHandle } from './provider.ts';
import { DEFAULT_FAMILY_THEME } from '../../draw/kit.ts';
import { fetchArtHandles, spriteResolutions } from './transport.ts';
import type { ArtHandle } from '../plan.ts';
import {
  artHandleKey,
  CURRENT_INSTALL_PROVENANCE,
  NO_ART,
  PENDING_ART,
  defaultArtFrame,
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

test('LaneArtCache distinguishes all six Rust ArtResolution states, wakes on load and failure, retries, and flushes on content revision', async () => {
  const closed: string[] = [];
  const aborted: string[] = [];
  let failOnce = true;
  let wakes = 0;

  const cache = new LaneArtCache<ArtBitmap>({
    maxEntries: 3,
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

  assert.deepEqual(cache.resolve(NO_ART), { status: 'noArtExpected' });
  assert.deepEqual(cache.resolve(PENDING_ART), { status: 'pending' });
  assert.deepEqual(cache.resolve(missingArt('not in mod stack')), { status: 'confirmedMissing', reason: 'not in mod stack' });
  assert.deepEqual(cache.resolve(refusedArt('unsupported DDS format', 'dds')), { status: 'refused', reason: 'unsupported DDS format', construct: 'dds' });
  assert.equal(cache.resolve(failedArt('rpc failed')).status, 'failed');

  const okRes = readyArt('art://ok', { frame: defaultArtFrame(41, 26, 2, 4), provenance: CURRENT_INSTALL_PROVENANCE });
  const flakyRes = readyArt('art://flaky');
  const slowRes = readyArt('art://slow');
  const flush = async () => { for (let i = 0; i < 4; i++) await Promise.resolve(); };

  cache.requestVisible([okRes, flakyRes, slowRes]);
  assert.equal(cache.resolve(okRes).status, 'pending');
  assert.equal(cache.resolve(flakyRes).status, 'pending');

  await flush();

  const ready = cache.resolve(okRes);
  assert.equal(ready.status, 'ready');
  assert.equal(ready.provenance.fromCurrentInstall, true);
  assert.equal(ready.frame.frameIndex, 2);
  assert.equal(ready.frame.frameCount, 4);
  assert.ok(ready.bitmap);

  const failed = cache.resolve(flakyRes);
  assert.equal(failed.status, 'failed');
  assert.equal(failed.reason, 'network timeout');
  assert.ok(wakes >= 2, 'wakes on both load and failure');

  failed.retry?.();
  await flush();
  assert.equal(cache.resolve(flakyRes).status, 'ready', 'retry re-fetches and decodes');

  cache.requestVisible([okRes, flakyRes]);
  assert.deepEqual(aborted, ['art://slow'], 'scrolled-off pending fetch is aborted');

  cache.requestVisible([readyArt('art://third'), readyArt('art://fourth')]);
  await flush();
  assert.equal(cache.cachedCount, 3);
  assert.ok(closed.includes('art://ok'), 'evicted bitmap is closed');

  cache.setContentRevision(2);
  assert.equal(cache.cachedCount, 0);
  assert.equal(cache.resolve(flakyRes).status, 'pending', 'content revision flush clears cache');

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
        onload: null as ((...args: any[]) => unknown) | null,
        onerror: null as ((...args: any[]) => unknown) | null,
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

  const res = readyArt('dom://icon1');
  let wakes = 0;
  const unsub = cache.subscribe('dom://icon1', () => { wakes++; });

  assert.equal(cache.resolve(res).status, 'pending');
  created[0]!.triggerError();
  assert.equal(wakes, 1, 'wakes subscriber on error');
  const failed = cache.resolve(res);
  assert.equal(failed.status, 'failed');

  failed.retry?.();
  assert.equal(cache.resolve(res).status, 'pending');
  created[1]!.triggerLoad();
  const ready = cache.resolve(res);
  assert.equal(ready.status, 'ready');
  assert.equal(ready.bitmap?.naturalWidth, 32);
  assert.equal(ready.provenance.label, 'Current installation');

  cache.setContentRevision(2);
  assert.equal(cache.cachedCount, 0);
  unsub();
});

test('at 1,500 assets the lane decodes a bounded few at a time and holds no more than its limit, the DOM cache keeps no failure past its limit, and a decode that lands before subscribing is seen', async () => {
  let started = 0;
  let inFlight = 0;
  let mostInFlight = 0;
  let open = 0;
  const lane = new LaneArtCache<ArtBitmap>({
    maxEntries: 128,
    fetchBitmap: () => {
      started++;
      inFlight++;
      mostInFlight = Math.max(mostInFlight, inFlight);
      return new Promise(resolve => queueMicrotask(() => {
        inFlight--;
        open++;
        const bitmap = fakeBitmap();
        bitmap.close = () => { open--; };
        resolve(bitmap);
      }));
    },
  });
  const urls = Array.from({ length: 1500 }, (_, i) => readyArt(`art://flag-${i}`));
  lane.requestVisible(urls);
  assert.ok(lane.pendingCount <= 6, 'decodes start a bounded few at a time');
  for (let i = 0; i < 2000; i++) await Promise.resolve();
  assert.ok(mostInFlight <= 6, 'never more than the concurrent bound in flight');
  assert.equal(started, 128, 'no decode starts that the cache could not keep');
  assert.equal(lane.cachedCount, 128);
  assert.equal(open, 128, 'every bitmap decoded is held or closed');
  assert.equal(lane.resolve(urls[1499]!).status, 'pending', 'art beyond the limit waits, never borrowed or dropped');

  lane.requestVisible(urls.slice(1400));
  for (let i = 0; i < 2000; i++) await Promise.resolve();
  assert.equal(started, 228);
  assert.equal(lane.cachedCount, 128);
  assert.equal(open, 128, 'art scrolled away is closed to make room');
  assert.equal(lane.resolve(urls[1499]!).status, 'ready');
  lane.dispose();
  assert.equal(open, 0);

  const images: (DomArtImage & { fail: () => void; load: () => void })[] = [];
  const dom = new DomArtCache<DomArtImage>({
    maxEntries: 2048,
    maxFailed: 2048,
    createImage: () => {
      const image = {
        complete: false,
        naturalWidth: 0,
        naturalHeight: 0,
        src: '',
        onload: null as ((...args: any[]) => unknown) | null,
        onerror: null as ((...args: any[]) => unknown) | null,
        fail() { this.complete = true; this.onerror?.(); },
        load() { this.complete = true; this.naturalWidth = 8; this.naturalHeight = 8; this.onload?.(); },
      };
      images.push(image);
      return image;
    },
  });
  for (let i = 0; i < 2050; i++) dom.resolve(readyArt(`dom://broken-${i}`));
  for (const image of images) image.fail();
  assert.equal(dom.failedCount, 2048, 'failures beyond the limit are evicted');
  assert.equal(dom.cachedCount, 0, 'a failed image is not also kept as an entry');

  assert.equal(dom.resolve(readyArt('dom://late')).status, 'pending');
  images.at(-1)!.load();
  assert.equal(dom.stateOfUrl('dom://late'), 'ready', 'a decode that settled before the subscriber arrived is seen when it subscribes');
});

test('ArtProvider plugs LaneArtCache and DomArtCache behind BoundedReadView.art() for flags and URLs', async () => {
  const index = new IdentityIndex({ sourceId: 's1', revision: 'r1', branchId: '1', content: 1 });
  const choice = { keys: ['GER_a', null, null] as [string | null, string | null, string | null], reasons: [null, null, null] as [string | null, string | null, string | null] };
  index.setDrawnFlag('panel', choice, { url: 'art://ger-flag' });
  const handle = flagArtHandle(flagChoiceKey(choice), 'panel');

  let wakes = 0;
  const laneCache = new LaneArtCache<ArtBitmap>({
    onSettle: () => { wakes++; },
    fetchBitmap: () => Promise.resolve(fakeBitmap()),
  });
  const demanded: string[] = [];
  const view = createBoundedReadView({
    hour: 100,
    art: createLaneArtProvider(laneCache as unknown as LaneArtCache<ImageBitmap>, { index }),
    onArtDemand: h => {
      const url = artUrlForHandle(h, index);
      if (url) demanded.push(url);
    },
  });

  const first = view.art(handle);
  assert.equal(first.status, 'pending');
  assert.deepEqual(demanded, ['art://ger-flag']);

  laneCache.requestVisible(demanded);
  for (let i = 0; i < 4; i++) await Promise.resolve();
  assert.equal(wakes, 1);

  const ready = view.art(handle);
  assert.equal(ready.status, 'ready');
  assert.equal(ready.url, 'art://ger-flag');
  assert.equal(ready.provenance.label, 'Current installation');
  assert.ok(ready.bitmap);

  const domCache = new DomArtCache<DomArtImage>({
    createImage: () => ({ complete: true, naturalWidth: 41, naturalHeight: 26, src: '', onload: null, onerror: null }),
  });
  const domView = createBoundedReadView({
    hour: 100,
    art: createDomArtProvider(domCache as unknown as DomArtCache<HTMLImageElement>),
  });
  const domReady = domView.art({ kind: 'url', url: 'https://art.test/icon.png' });
  assert.equal(domReady.status, 'ready');
  assert.equal(domReady.url, 'https://art.test/icon.png');
  assert.equal(domReady.provenance.label, 'Current installation');

  laneCache.dispose();
});

test('Rust ArtResolution on CountryFlagImage and SpriteImage passes straight through adapters, caches, and spriteResolutions', () => {
  const modOrigin = { kind: 'mod', id: '2777392649', name: 'Millennium Dawn' } as const;
  const modProvenance = { fromCurrentInstall: true, label: 'Current installation (Millennium Dawn)', scopeKey: 'md' };
  const readyModRes = {
    status: 'ready',
    assetId: 'md:flag:USA',
    url: 'asset://md/flags/USA.png',
    origin: modOrigin,
    file: 'gfx/flags/USA.tga',
    frame: defaultArtFrame(82, 52),
    provenance: modProvenance,
  } as const;
  const missingSpiritRes = {
    status: 'confirmedMissing',
    reason: 'Spirit declares no sprite in mod stack',
    origin: modOrigin,
    file: 'common/ideas/md.txt',
  } as const;

  const flagImage = {
    flag: 0,
    asset: 'gfx/flags/USA.tga',
    key: 'md:flag:USA',
    width: 82,
    height: 52,
    png: 'asset://md/flags/USA.png',
    resolution: readyModRes,
  };
  assert.strictEqual(flagImageToArt(flagImage), readyModRes);
  assert.strictEqual(flagEntryToArt({ url: flagImage.png, image: flagImage }), readyModRes);

  const spriteImage = { key: 'GFX_md_spirit_none', frame: 0, icon: null, resolution: missingSpiritRes };
  assert.strictEqual(spriteImageToArt(spriteImage), missingSpiritRes);

  const parsed = spriteResolutions(
    {
      items: [
        { key: 'GFX_md_spirit_ok', frame: 0, icon: 'asset://md/spirit.png', resolution: readyModRes },
        spriteImage,
      ],
      warnings: [],
    },
    [{ key: 'GFX_md_spirit_ok', frame: 0 }, { key: 'GFX_md_spirit_none', frame: 0 }],
  );
  const okKey = artHandleKey({ kind: 'sprite', key: 'GFX_md_spirit_ok', frame: 0 });
  const missingKey = artHandleKey({ kind: 'sprite', key: 'GFX_md_spirit_none', frame: 0 });
  assert.deepEqual(parsed.get(okKey), readyModRes);
  assert.deepEqual(parsed.get(missingKey), missingSpiritRes);
});

test('sprite, portrait, technology and relative-path handles are asked of their products and drawn from the answer by the lane', async () => {
  const sprite: ArtHandle = { kind: 'sprite', key: 'GFX_x', frame: 0, text: true };
  const portrait: ArtHandle = { kind: 'portrait', leader: '12', hour: 300 };
  const path: ArtHandle = { kind: 'path', path: 'gfx/interface/icon.dds', frame: 1, frames: 2 };
  const radar: ArtHandle = { kind: 'technology', token: 'radar', databaseIndex: 7 };
  const unjoined: ArtHandle = { kind: 'technology', token: 'mod_tech', databaseIndex: null };
  const absent: ArtHandle = { kind: 'technology', token: 'gone_tech', databaseIndex: 3 };
  const asks = new Map<string, ArtHandle>();
  const urls = new Set<string>();
  for (const handle of [sprite, portrait, path, radar, unjoined, absent, { kind: 'url', url: 'https://art.test/a.png' } as const]) laneArtDemand(handle, null, undefined, urls, asks);
  assert.deepEqual([...asks.values()], [sprite, portrait, path, radar, unjoined, absent]);
  assert.deepEqual([...urls], ['https://art.test/a.png']);

  const requests: { product: string; params: unknown }[] = [];
  const ready = readyArt('art://drawn');
  const missing = missingArt('The game generates this portrait from unrecorded inputs');
  const answered = await fetchArtHandles(async request => {
    requests.push({ product: request.product, params: request.params });
    if (request.product === 'unit-leader-portraits') return { portraits: [{ reference: '12', image: null, missing: missing.reason, resolution: missing }], warnings: [] };
    if (request.product === 'technology-artwork') {
      return { items: [
        { token: 'radar', name: 'Radar', description: null, baseDurationDays: 100, icon: 'http://artwork.localhost/0123456789abcdef0123456789abcdef.png' },
        { token: 'mod_tech', name: 'Mod technology', description: null, baseDurationDays: null, icon: null },
      ], warnings: [] };
    }
    return {
      items: [{ key: 'GFX_x', frame: 0, text: true, icon: 'art://drawn', resolution: ready }],
      textures: [{ path: 'gfx/interface/icon.dds', frame: 1, frames: 2, resolution: ready }],
      warnings: [],
    };
  }, { source: { id: 's', revision: 'r' }, branch: { id: '1' } } as never, [...asks.values()], new AbortController().signal);
  assert.deepEqual(requests.map(request => request.product).sort(), ['sprite-artwork', 'technology-artwork', 'unit-leader-portraits']);
  assert.deepEqual(requests.find(request => request.product === 'technology-artwork')!.params,
    { tokens: ['gone_tech', 'mod_tech', 'radar'], recorded: { gone_tech: [3], radar: [7] } }, 'one request; a definition read without a recorded index asks by token');
  assert.deepEqual(requests.find(request => request.product === 'sprite-artwork')!.params, {
    sprites: [{ key: 'GFX_x', frame: 0, text: true }],
    textures: [{ path: 'gfx/interface/icon.dds', frame: 1, frames: 2 }],
  });
  const resolutions = Object.fromEntries(answered);
  assert.deepEqual(resolutions[artHandleKey(portrait)], missing);
  assert.deepEqual(resolutions[artHandleKey(path)], ready);
  assert.equal(resolutions[artHandleKey(radar)]?.status, 'ready');
  assert.deepEqual(resolutions[artHandleKey(unjoined)], missingArt('Art missing from the current installation'));
  assert.deepEqual(resolutions[artHandleKey(absent)], missingArt('Technology artwork was not returned for this technology'));

  const lane = createLaneArtProvider(new LaneArtCache(), { resolutions });
  assert.equal(lane.resolve(portrait).status, 'confirmedMissing');
  assert.equal(artUrlForHandle(path, null, resolutions), 'art://drawn');
  const again = new Map<string, ArtHandle>();
  laneArtDemand(path, null, resolutions, urls, again);
  assert.equal(again.size, 0);
  assert.ok(urls.has('art://drawn'));
});

test('urlArtHandle, collectArtUrls, resolveCachedArt, and drawMediaSlot unify family media handling', () => {
  assert.equal(urlArtHandle(null), null);
  assert.equal(urlArtHandle(''), null);
  assert.deepEqual(urlArtHandle('art://icon'), { kind: 'url', url: 'art://icon' });

  const collected = collectArtUrls([
    null,
    undefined,
    'art://direct',
    urlArtHandle('art://handle'),
    readyArt('art://ready'),
    PENDING_ART,
    { iconUrl: 'art://direct' },
    { art: readyArt('art://slot') },
  ]);
  assert.deepEqual(collected, ['art://direct', 'art://handle', 'art://ready', 'art://slot']);

  const bmp = fakeBitmap() as unknown as ImageBitmap;
  const icons = new Map<string, ImageBitmap | null>([['art://cached', bmp]]);
  const resolved = resolveCachedArt(readyArt('art://cached'), { icons });
  assert.equal(resolved.status, 'ready');
  assert.equal(resolved.bitmap, bmp);

  const ops: string[] = [];
  const ctx = {
    save() { ops.push('save'); },
    restore() { ops.push('restore'); },
    beginPath() {},
    roundRect() {},
    fill() { ops.push('fill'); },
    stroke() { ops.push('stroke'); },
    clip() {},
    drawImage() { ops.push('drawImage'); },
    fillText() { ops.push('fillText'); },
    measureText: (t: string) => ({ width: t.length * 6 }),
    font: '',
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    globalAlpha: 1,
    textAlign: 'left',
    textBaseline: 'middle',
  } as unknown as CanvasRenderingContext2D;

  drawMediaSlot(ctx, 4, 14, 18, 12, { kind: 'flag', art: readyArt('art://cached'), tag: 'GER' }, DEFAULT_FAMILY_THEME, { icons });
  drawMediaSlot(ctx, 4, 14, 18, 18, { kind: 'art', art: readyArt('art://cached'), name: 'Tank' }, DEFAULT_FAMILY_THEME, { icons, radius: 3 });
  assert.equal(ops.filter(op => op === 'drawImage').length, 2);
});

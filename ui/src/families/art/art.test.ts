import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../boundedView.ts';
import { IdentityIndex } from '../identity/index.ts';
import { flagChoiceKey, parseFlagChoiceKey } from '../identity/types.ts';
import { DomArtCache, type DomArtImage } from './domCache.ts';
import { LaneArtCache } from './laneCache.ts';
import { drawMediaSlot, flagEntryToArt, resolveCachedArt, spriteImageToArt } from './adapters.ts';
import { artUrlForHandle, collectArtUrls, createDomArtProvider, createLaneArtProvider, flagArtHandle, laneArtDemand, urlArtHandle } from './provider.ts';
import { DEFAULT_FAMILY_THEME } from '../../draw/kit.ts';
import { MAX_ART_CALLS, SpriteCaches, artCacheScope, fetchArtHandles, spriteResolutions } from './transport.ts';
import type { ArtHandle } from '../plan.ts';
import {
  artHandleKey,
  CURRENT_INSTALL_PROVENANCE,
  NO_ART,
  PENDING_ART,
  defaultArtFrame,
  failedArt,
  productArt,
  readyArt,
  refusedArt,
  type ArtBitmap,
} from './types.ts';
import type { CountryFlagChoice } from '../../generated/dashboard';

const missingArt = (reason: string) => productArt({ status: 'confirmedMissing', reason });

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
  assert.deepEqual([cache.resolve(okRes).status, 'bitmap' in cache.resolve(okRes)], ['ready', false], 'undecoded art keeps its address and has no bitmap yet');
  assert.deepEqual([cache.resolve(flakyRes).status, 'bitmap' in cache.resolve(flakyRes)], ['ready', false]);

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
  assert.equal(cache.stateOfUrl(flakyRes.url), 'unrequested', 'content revision flush clears cache');

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
  lane.requestVisible([], urls);
  assert.ok(lane.pendingCount <= 6, 'decodes start a bounded few at a time');
  for (let i = 0; i < 2000; i++) await Promise.resolve();
  assert.ok(mostInFlight <= 6, 'never more than the concurrent bound in flight');
  assert.equal(started, 128, 'no decode starts that the cache could not keep');
  assert.equal(lane.cachedCount, 128);
  assert.equal(open, 128, 'every bitmap decoded is held or closed');
  assert.equal(lane.stateOfUrl(urls[1499]!.url), 'unrequested', 'offscreen art beyond the limit waits, never borrowed or dropped');

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
  assert.deepEqual([first.status, 'bitmap' in first], ['ready', false], 'an undecoded flag keeps its address, so the painted window knows it before any decode');
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
  const missing = { status: 'unavailable', reason: 'The game generates this portrait from unrecorded inputs' } as const;
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
  assert.deepEqual(resolutions[artHandleKey(unjoined)], failedArt('The technology artwork product drew no icon and does not say why', false));
  assert.deepEqual(resolutions[artHandleKey(absent)], failedArt('The technology artwork reply left this technology out', false));

  const lane = createLaneArtProvider(new LaneArtCache(), { resolutions });
  assert.equal(lane.resolve(portrait).status, 'unavailable');
  assert.equal(artUrlForHandle(path, null, resolutions), 'art://drawn');
  const again = new Map<string, ArtHandle>();
  laneArtDemand(path, null, resolutions, urls, again);
  assert.equal(again.size, 0);
  assert.ok(urls.has('art://drawn'));
});

test('urlArtHandle, collectArtUrls, resolveCachedArt, and drawMediaSlot unify family media handling', async () => {
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
  const cache = new LaneArtCache({ fetchBitmap: async () => bmp });
  cache.requestVisible(['art://cached']);
  for (let i = 0; i < 20; i++) await Promise.resolve();
  const resolved = resolveCachedArt(readyArt('art://cached'), cache);
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

  drawMediaSlot(ctx, 4, 14, 18, 12, { kind: 'flag', art: readyArt('art://cached'), tag: 'GER' }, DEFAULT_FAMILY_THEME, { artCache: cache });
  drawMediaSlot(ctx, 4, 14, 18, 18, { kind: 'art', art: readyArt('art://cached'), name: 'Tank' }, DEFAULT_FAMILY_THEME, { artCache: cache, radius: 3 });
  assert.equal(ops.filter(op => op === 'drawImage').length, 2);
});

test('a completed search travels with the flag choice to the flag product', () => {
  const relayed: CountryFlagChoice = { keys: [null, 'aaa', 'aaa'], reasons: ['none drawn', null, null], searched: [true, null, null] };
  assert.deepEqual(parseFlagChoiceKey(flagChoiceKey(relayed)), relayed);
  assert.notEqual(flagChoiceKey(relayed), flagChoiceKey({ ...relayed, searched: undefined }));
});

test('a portrait is cached per branch: two branches with the same leader reference and hour draw their own portrait', async () => {
  const caches = new SpriteCaches();
  const portrait: ArtHandle = { kind: 'portrait', leader: '12', hour: 300 };
  const tracks = ['1', '2'].map(id => ({ source: { id: 's', revision: 'r' }, branch: { id } }) as never as Parameters<typeof artCacheScope>[0]);
  const asked: string[] = [];
  for (const track of tracks) {
    const scope = artCacheScope(track, 4);
    caches.hold(scope);
    caches.request(scope, [portrait], (batch, signal) => fetchArtHandles(async request => {
      const params = request.params as { branchId: string };
      asked.push(params.branchId);
      return { portraits: [{ reference: '12', image: `art://branch-${params.branchId}`, missing: null, resolution: readyArt(`art://branch-${params.branchId}`) }], warnings: [] };
    }, track as never, batch, signal));
  }
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.deepEqual(asked.sort(), ['1', '2'], 'each branch asks its own portrait');
  const shown = tracks.map(track => caches.resolutions(artCacheScope(track, 4)).get(artHandleKey(portrait)));
  assert.deepEqual(shown.map(art => art?.status === 'ready' ? art.url : null), ['art://branch-1', 'art://branch-2']);
  assert.notEqual(artCacheScope(tracks[0]!, 4), artCacheScope(tracks[0]!, 5), 'a content revision (mod stack or presentation) is its own scope');
});

test('portrait calls are bounded and settle one by one; a retryable failure is retried per handle after its backoff and a refusal stays', async () => {
  const portraits: ArtHandle[] = Array.from({ length: 64 }, (_, hour) => ({ kind: 'portrait', leader: '12', hour: 100 + hour }));
  let open = 0;
  let most = 0;
  const calls: number[] = [];
  const answered = await fetchArtHandles(async request => {
    const hour = (request.params as { hour: number }).hour;
    calls.push(hour);
    open++;
    most = Math.max(most, open);
    await Promise.resolve();
    open--;
    if (hour === 103) throw new Error('Request timed out');
    return { portraits: [{ reference: '12', image: `art://${hour}`, missing: null, resolution: readyArt(`art://${hour}`) }], warnings: [] };
  }, { source: { id: 's', revision: 'r' }, branch: { id: '1' } } as never, portraits, new AbortController().signal);
  assert.equal(calls.length, 64);
  assert.ok(most <= MAX_ART_CALLS, `at most ${MAX_ART_CALLS} product calls at once, saw ${most}`);
  assert.equal([...answered.values()].filter(art => art.status === 'ready').length, 63, 'one rejected call discards none of its siblings');
  assert.equal(answered.get(artHandleKey(portraits[3]!))?.status, 'failed');

  let now = 0;
  const timers: { at: number; run: () => void }[] = [];
  const caches = new SpriteCaches({ now: () => now, after: (ms, run) => { const timer = { at: now + ms, run }; timers.push(timer); return () => { timers.splice(timers.indexOf(timer), 1); }; } });
  const flaky: ArtHandle = { kind: 'sprite', key: 'GFX_flaky', frame: 0 };
  const refused: ArtHandle = { kind: 'sprite', key: 'GFX_refused', frame: 0 };
  let attempts = 0;
  const fetch = async (batch: readonly ArtHandle[]) => new Map(batch.map(handle => [artHandleKey(handle), handle === refused || (handle.kind === 'sprite' && handle.key === 'GFX_refused')
    ? refusedArt('unsupported sprite', 'sprite-render')
    : ++attempts === 1 ? failedArt('transport dropped', true) : readyArt('art://flaky')]));
  caches.hold('scope');
  caches.request('scope', [flaky, refused], fetch);
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.equal(caches.failures('scope').get(artHandleKey(flaky))?.retryable, true);
  assert.equal(caches.resolutions('scope').get(artHandleKey(refused))?.status, 'refused', 'a refusal is settled, never retried');
  caches.request('scope', [flaky], fetch);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  assert.equal(attempts, 1, 'no new request inside the backoff');
  now = timers[0]!.at;
  timers.shift()!.run();
  for (let i = 0; i < 6; i++) await Promise.resolve();
  assert.equal(attempts, 2, 'the failed handle is asked again once its backoff ends');
  assert.equal(caches.resolutions('scope').get(artHandleKey(flaky))?.status, 'ready');
  assert.equal(caches.failures('scope').size, 0);
});

test('the lane pins and fetches every visible bitmap past its budget; offscreen demand stays inside the budget; a paused lane fetches and wakes nothing', async () => {
  const fetched: string[] = [];
  let settled = 0;
  const cache = new LaneArtCache({
    maxEntries: 128,
    maxConcurrent: 6,
    onSettle: () => { settled++; },
    fetchBitmap: async url => { fetched.push(url); await Promise.resolve(); return fakeBitmap(); },
  });
  const flush = async () => { for (let i = 0; i < 400; i++) await Promise.resolve(); };
  const visible = Array.from({ length: 200 }, (_, at) => `art://flag-${at}`);
  for (let round = 0; round < 3; round++) { cache.requestVisible(visible); await flush(); }
  assert.equal(visible.filter(url => cache.stateOfUrl(url) === 'ready').length, 200, 'the 129th and later visible images load without a scroll');
  const afterVisible = { fetched: fetched.length, settled };
  for (let paint = 0; paint < 30; paint++) { cache.requestVisible(visible); await flush(); }
  assert.deepEqual({ fetched: fetched.length, settled }, afterVisible, 'once the visible set is resident a paused lane asks and wakes nothing');

  const offscreen = Array.from({ length: 1_500 }, (_, at) => `art://row-${at}`);
  const few = visible.slice(0, 10);
  for (let paint = 0; paint < 20; paint++) { cache.requestVisible(few, offscreen); await flush(); }
  const offscreenFetches = fetched.filter(url => url.startsWith('art://row-')).length;
  assert.ok(offscreenFetches <= 128, `offscreen demand never exceeds the budget (${offscreenFetches})`);
  const before = fetched.length;
  for (let paint = 0; paint < 20; paint++) { cache.requestVisible(few, offscreen); await flush(); }
  assert.equal(fetched.length, before, 'a demanded bitmap is never evicted to admit another, so repeated paints fetch nothing');
  assert.ok(cache.cachedCount <= 138);
});

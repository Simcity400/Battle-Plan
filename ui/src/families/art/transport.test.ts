import test from 'node:test';
import assert from 'node:assert/strict';
import { SPRITE_BATCH, SpriteCaches, spriteFrames, spriteHandle, spriteResolutions, type SpriteFrame } from './transport.ts';
import { artHandleKey, readyArt, type ArtResolution } from './types.ts';
import type { ArtHandle } from '../plan.ts';

const spriteId = (sprite: SpriteFrame) => artHandleKey(spriteHandle(sprite));
const urlOf = (resolution: ArtResolution | undefined) => resolution?.status === 'ready' ? resolution.url : null;
const spriteIcons = (reply: unknown, asked: readonly SpriteFrame[]) => {
  const icons = new Map<string, string | null>();
  for (const [id, resolution] of spriteResolutions(reply, asked)) icons.set(id, urlOf(resolution));
  return icons;
};

const png = 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png';

test('sprite frames are asked once each, in a stable order, without missing keys', () => {
  assert.deepEqual(spriteFrames([{ key: 'GFX_b', frame: 0 }, null, { key: 'GFX_a', frame: 2 }, undefined, { key: '', frame: 0 },
    { key: 'GFX_b', frame: 0 }, { key: 'GFX_a', frame: -1 }]), [{ key: 'GFX_a', frame: 2 }, { key: 'GFX_b', frame: 0 }]);
});

test('sprite art maps each asked frame to its drawable image or null', () => {
  const asked = [{ key: 'GFX_a', frame: 0 }, { key: 'GFX_a', frame: 3 }, { key: 'GFX_c', frame: 0 }];
  const icons = spriteIcons({ items: [{ key: 'GFX_a', frame: 0, icon: png }, { key: 'GFX_a', frame: 3, icon: null },
    { key: 'GFX_c', frame: 0, icon: 'https://example.invalid/x.png' }], warnings: [] }, asked);
  assert.deepEqual(asked.map(sprite => icons.get(spriteId(sprite))), [png, null, null]);
});

test('a frame with the recording\'s own choice is asked and answered apart from the same frame without one', () => {
  assert.deepEqual(spriteFrames([{ key: 'GFX_b', frame: 1, recorded: 'GFX_a' }, { key: 'GFX_b', frame: 1 },
    { key: 'GFX_b', frame: 1, recorded: '' }]), [{ key: 'GFX_b', frame: 1, recorded: 'GFX_a' }, { key: 'GFX_b', frame: 1 }]);
  const asked = [{ key: 'GFX_b', frame: 1, recorded: 'GFX_a' }, { key: 'GFX_b', frame: 1 }];
  const icons = spriteIcons({ items: [{ key: 'GFX_b', frame: 1, recorded: 'GFX_a', icon: png },
    { key: 'GFX_b', frame: 1, icon: null }], warnings: [] }, asked);
  assert.deepEqual(asked.map(sprite => icons.get(spriteId(sprite))), [png, null]);
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_b', frame: 1, recorded: 'GFX_c', icon: png }], warnings: [] }, asked), /malformed/u);
});

test('a text icon is asked and answered apart from the sprite of the same key', () => {
  const asked = spriteFrames([{ key: 'pol_power', frame: 0, text: true }, { key: 'pol_power', frame: 0 }]);
  assert.equal(asked.length, 2);
  const icons = spriteIcons({ items: [{ key: 'pol_power', frame: 0, text: true, icon: png }, { key: 'pol_power', frame: 0, icon: null }], warnings: [] }, asked);
  assert.equal(icons.get(spriteId({ key: 'pol_power', frame: 0, text: true })), png);
  assert.equal(icons.get(spriteId({ key: 'pol_power', frame: 0 })), null);
});

test('frame ids keep their parts apart whatever the keys hold', () => {
  assert.notEqual(spriteId({ key: 'GFX_a:x', frame: 0, recorded: 'GFX_b' }), spriteId({ key: 'GFX_a', frame: 0, recorded: 'x:GFX_b' }));
  assert.notEqual(spriteId({ key: 'GFX_a', frame: 0 }), spriteId({ key: 'GFX_a', frame: 0, recorded: 'null' }));
});

test('a sprite reply answering frames it was not asked for is malformed', () => {
  const asked = [{ key: 'GFX_a', frame: 0 }];
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_a', frame: 1, icon: png }], warnings: [] }, asked), /malformed/u);
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_a', frame: 0, icon: png }, { key: 'GFX_a', frame: 0, icon: png }], warnings: [] }, asked), /malformed/u);
  assert.throws(() => spriteIcons({ items: [] }, asked), /malformed/u);
});

test('each active recording and revision keeps its own icons, and a scope no one shows is dropped', () => {
  const caches = new SpriteCaches();
  const releaseA = caches.hold('a'), releaseB = caches.hold('b');
  caches.resolutions('a').set('icon', readyArt(png));
  caches.resolutions('b').set('icon', readyArt(png));
  assert.equal(urlOf(caches.resolutions('a').get('icon')), png, 'another recording shown at the same time never evicts these icons');
  releaseA();
  assert.equal(urlOf(caches.resolutions('b').get('icon')), png);
  assert.equal(caches.resolutions('a').has('icon'), false, 'a scope released by its last widget is not kept');
  releaseB();
});

test('two recordings mounting together keep their own icons whatever order their renders and effects run in', () => {
  const caches = new SpriteCaches();
  const a = caches.resolutions('a'), b = caches.resolutions('b');
  const releaseA = caches.hold('a');
  const releaseB = caches.hold('b');
  assert.equal(caches.resolutions('a'), a);
  assert.equal(caches.resolutions('b'), b, 'a scope rendered but not yet held is not dropped when another is held');
  b.set('icon', readyArt(png));
  assert.equal(urlOf(caches.resolutions('b').get('icon')), png);
  releaseA(); releaseB();
});

test('sprite requests from multiple callers in the same tick coalesce into bounded batches without duplicate or cancelled calls', async () => {
  const caches = new SpriteCaches();
  const release = caches.hold('s');
  const calls: number[] = [];
  let aborted = 0;
  const fetcher = async (batch: readonly ArtHandle[], signal: AbortSignal) => {
    calls.push(batch.length);
    signal.addEventListener('abort', () => { aborted++; });
    await Promise.resolve();
    return new Map(batch.map(handle => [artHandleKey(handle), readyArt(png)] as const));
  };
  const first = Array.from({ length: 10 }, (_, i) => ({ key: `GFX_${i}`, frame: 0 }));
  const second = Array.from({ length: 10 }, (_, i) => ({ key: `GFX_${i + 5}`, frame: 0 }));
  const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  caches.request('s', first.map(spriteHandle), fetcher);
  caches.request('s', second.map(spriteHandle), fetcher);
  await flush();
  assert.deepEqual(calls, [15]);
  assert.equal(aborted, 0);
  assert.equal(urlOf(caches.resolutions('s').get(spriteId({ key: 'GFX_14', frame: 0 }))), png);
  assert.equal(caches.resolutions('s').get(spriteId({ key: 'GFX_14', frame: 0 }))?.status, 'ready');
  const large = Array.from({ length: SPRITE_BATCH + 10 }, (_, i) => ({ key: `GFX_L_${i}`, frame: 0 }));
  caches.request('s', large.map(spriteHandle), fetcher);
  await flush();
  assert.deepEqual(calls, [15, SPRITE_BATCH, 10]);
  release();
});

test('failed sprite batches record errors without marking sprites as missing and notify watchers', async () => {
  const caches = new SpriteCaches();
  const release = caches.hold('err');
  let notified = 0;
  const stop = caches.watch('err', () => { notified++; });
  const failing = async () => { throw new Error('disk offline'); };
  caches.request('err', [spriteHandle({ key: 'GFX_fail', frame: 0 })], failing);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  const id = spriteId({ key: 'GFX_fail', frame: 0 });
  assert.equal(caches.errors('err').get(id), 'disk offline');
  assert.equal(caches.resolutions('err').has(id), false);
  assert.equal(notified, 1);
  stop();
  release();
});

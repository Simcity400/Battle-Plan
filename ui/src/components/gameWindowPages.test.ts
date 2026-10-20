import assert from 'node:assert/strict';
import test from 'node:test';
import { PageCache } from './gameWindowPages.ts';

const page = (id: number, bytes: number) => ({ id, width: 1, height: 1, image: 'x'.repeat(bytes) });

test('pages stay within their budget, letting the least recently used released sessions go first', () => {
  const revoked: string[] = [];
  const cache = new PageCache(100, url => revoked.push(url));
  cache.acquire('a'); cache.add('a', page(1, 40)); cache.addTinted('a', '1|0.5,0.5,0.5', 'blob:a', 10);
  cache.acquire('b'); cache.add('b', page(1, 40));
  assert.equal(cache.bytes(), 90);
  // Over budget while both are drawn: nothing in use is dropped.
  cache.acquire('c'); cache.add('c', page(1, 40));
  assert.equal(cache.bytes(), 130);
  assert.ok(cache.has('a') && cache.has('b') && cache.has('c'));
  // Released, the least recently used goes first, with its coloured pages revoked.
  cache.release('b'); cache.release('a');
  assert.ok(!cache.has('a') !== !cache.has('b'));
  assert.ok(cache.bytes() <= 100);
  cache.release('c');
  assert.ok(cache.bytes() <= 100);
  // A coloured page arriving for a session let go is revoked, not kept.
  const gone = cache.has('a') ? 'b' : 'a';
  assert.equal(cache.addTinted(gone, '1|1,0,0', 'blob:late', 5), false);
  assert.ok(revoked.includes('blob:late'));
});

test('every window of an owner hears of a page another loaded or coloured, and no other owner does', () => {
  const cache = new PageCache(1000, () => {});
  const heard = [0, 0, 0];
  const stops = [cache.subscribe('a', () => { heard[0]++; }), cache.subscribe('a', () => { heard[1]++; }), cache.subscribe('b', () => { heard[2]++; })];
  const before = cache.revision('a');
  cache.acquire('a'); cache.add('a', page(1, 10));
  assert.ok(cache.addTinted('a', '1|1,0.5,0.5', 'blob:first', 4));
  // The second copy of the same colour is revoked, yet both windows already heard of the first.
  assert.equal(cache.addTinted('a', '1|1,0.5,0.5', 'blob:second', 4), false);
  assert.deepEqual(heard, [2, 2, 0], 'another owner\u2019s window does not redraw');
  assert.equal(cache.revision('a'), before + 2);
  assert.equal(cache.revision('b'), 0);
  for (const stop of stops) stop();
  cache.add('a', page(2, 10));
  assert.deepEqual(heard, [2, 2, 0], 'a window that stopped following hears nothing');
});

test('an owner\u2019s revision goes once it holds no pages and no window follows it', () => {
  const cache = new PageCache(50, () => {});
  for (let owner = 0; owner < 1000; owner++) {
    const key = `owner-${owner}`;
    const stop = cache.subscribe(key, () => {});
    cache.acquire(key); cache.add(key, page(1, 40)); cache.release(key);
    stop();
  }
  // Only the last owner still holds pages: every evicted, unfollowed owner left no revision.
  assert.equal(cache.trackedOwners(), 1);
  // A followed owner hears its eviction, and keeps its revision until it is no longer followed.
  let heard = 0;
  const stop = cache.subscribe('owner-999', () => { heard++; });
  cache.acquire('next'); cache.add('next', page(1, 40)); cache.release('next');
  assert.equal(heard, 1);
  assert.ok(!cache.has('owner-999'));
  assert.equal(cache.trackedOwners(), 2);
  stop();
  assert.equal(cache.trackedOwners(), 1);
});

test('a session no window draws is kept for reuse while the budget allows', () => {
  const cache = new PageCache(1000, () => {});
  cache.acquire('a'); cache.add('a', page(1, 40)); cache.release('a');
  assert.ok(cache.has('a'));
  assert.equal(cache.pages('a').get(1)?.image?.length, 40);
  // Replacing a page charges only the difference.
  cache.add('a', page(1, 10));
  assert.equal(cache.bytes(), 10);
});

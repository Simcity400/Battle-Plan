import assert from 'node:assert/strict';
import test from 'node:test';
import { KeyedReads } from './keyedReads.ts';

type Place = { provinceId: number; hour: number };
const key = (place: Place) => JSON.stringify([place.provinceId, place.hour]);
const named = (places: readonly Place[]) => ({ values: places.map(place => [key(place), `Province ${place.provinceId}`] as const), warnings: [] });

test('a list opened inside a window has its own places named when their read lands, though the next window asks before it lands (run 12: every place loading)', () => {
  const reads = new KeyedReads<Place, string>(key, 256);
  const head = [{ provinceId: 1, hour: 100 }, { provinceId: 2, hour: 140 }, { provinceId: 1, hour: 100 }];
  const first = reads.take(head);
  assert.deepEqual(first, [[{ provinceId: 1, hour: 100 }, { provinceId: 2, hour: 140 }]], 'the head window\'s places, each once');
  assert.ok(head.every(place => reads.pending(place)), 'loading while read');
  const ahead = [...head, { provinceId: 3, hour: 900 }];
  assert.deepEqual(reads.take(ahead), [[{ provinceId: 3, hour: 900 }]], 'nothing asked again, nothing cancelled');
  reads.landed(first[0]!, named(first[0]!));
  assert.equal(reads.values.get(key({ provinceId: 2, hour: 140 })), 'Province 2', 'the head window\'s rows are named');
  assert.equal(reads.pending({ provinceId: 1, hour: 100 }), false);
  assert.equal(reads.pending({ provinceId: 3, hour: 900 }), true, 'the window ahead still loading');
  for (let block = 0; block < 10; block++) assert.deepEqual(reads.take(ahead), [], 'never asked again per block or hour');
});

test('a key the reply leaves out is settled as unanswered, a failed read names its failure, and asks come in chunks', () => {
  const reads = new KeyedReads<Place, string>(key, 2);
  const places = [1, 2, 3].map(provinceId => ({ provinceId, hour: 10 }));
  const chunks = reads.take(places);
  assert.deepEqual(chunks.map(chunk => chunk.length), [2, 1]);
  reads.landed(chunks[0]!, named([places[0]!]));
  assert.equal(reads.pending(places[1]!), false, 'not answered: never loading for ever');
  assert.equal(reads.values.has(key(places[1]!)), false);
  reads.failed(chunks[1]!, new Error('read refused'));
  assert.equal(reads.failure(places[2]!)?.message, 'read refused');
  assert.equal(reads.pending(places[2]!), false);
  assert.deepEqual(reads.take(places), [], 'neither is asked again');
  const led = new KeyedReads<Place, string>(key, 3, 2);
  const many = [1, 2, 3, 4, 5, 6].map(provinceId => ({ provinceId, hour: 10 }));
  assert.deepEqual(led.take(many).map(chunk => chunk.map(place => place.provinceId)), [[1, 2], [3, 4, 5], [6]]);
});

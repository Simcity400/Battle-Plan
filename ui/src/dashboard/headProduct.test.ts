import assert from 'node:assert/strict';
import test from 'node:test';
import { headRead, headSource, headSubject, momentHour, retain } from './headProduct.ts';
import type { BoundTrack, WidgetSpec } from './model';

const track = { id: 'main', axisOffset: 10, branch: { id: 'b', firstHour: 100, lastHour: 500 } } as unknown as BoundTrack;
const widget = (time: WidgetSpec['time']) => ({ kind: 'naval-yards', trackIds: ['main'], time }) as Pick<WidgetSpec, 'time' | 'kind' | 'trackIds'>;

test('a widget following playback is followed; pinned, on a range or hovered, it asks its own hour', () => {
  const following = headRead(widget({ mode: 'follow' }), track, [track], null);
  assert.equal(following.follows, true);
  assert.equal(following.hourAt(200.7), 210, 'its fallback asks the clock’s hour on the track');
  const hovered = headRead(widget({ mode: 'follow' }), track, [track], 321);
  assert.deepEqual([hovered.follows, hovered.hourAt(200)], [false, 321], 'a hovered hour is asked whatever the head does');
  const pinned = headRead(widget({ mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 50 } } }), track, [track], null);
  assert.deepEqual([pinned.follows, pinned.hourAt(200), pinned.hourAt(400)], [false, 150, 150], 'a pinned hour never moves with the clock');
  const range = headRead(widget({ mode: 'range', firstOffset: 0, lastOffset: 80 }), track, [track], null);
  assert.deepEqual([range.follows, range.hourAt(0)], [false, 180], 'a range is read at its end');
});

test('an hour outside the recorded branch, or no track, asks nothing', () => {
  const following = headRead(widget({ mode: 'follow' }), track, [track], null);
  assert.equal(following.hourAt(50), null);
  assert.equal(following.hourAt(491), null);
  assert.equal(headRead(widget({ mode: 'follow' }), undefined, [], null).hourAt(200), null);
});

test('a followed panel whose first push has not landed is asked by the fallback’s clock, never held at an earlier hour', () => {
  assert.equal(headSource(true, 'followed', false), 'fallback', 'a seek before the first push asks the hour sought');
  assert.equal(headSource(true, 'followed', true), 'pushed');
  assert.equal(headSource(true, 'pending', false), 'fallback');
  assert.equal(headSource(true, 'unavailable', true), 'fallback');
  assert.equal(headSource(true, 'off', false), 'asked', 'not watched yet, or with no element, it is asked as rendered');
  assert.equal(headSource(true, 'off', false, true), 'idle', 'following but seen off screen, nothing is read for it, whatever renders around it');
  assert.equal(headSource(false, 'off', false, true), 'asked', 'not following, its own hour is asked wherever it is');
  assert.equal(headSource(false, 'followed', true), 'asked');
});

test('the reply shown while a newer hour loads is dated at the hour it was asked for', () => {
  assert.equal(momentHour(JSON.stringify([60_759_371])), 60_759_371);
  assert.equal(momentHour(undefined), null);
  assert.equal(momentHour(JSON.stringify([null])), null);
});

test('the reply kept while a push is on the way is of one history, product and params, and dropped when any changes', () => {
  const client = {}, other = {};
  const history = (source: string, revision: number, branch: string) => ({ source: { id: source, revision }, branch: { id: branch } }) as unknown as BoundTrack;
  const a = headSubject(history('s', 1, 'b'), 'aces', {});
  const kept = retain(null, client, a, 'A roster', 150);
  assert.deepEqual(retain(kept, client, a, undefined, null), kept, 'kept while the same subject loads');
  for (const subject of [headSubject(history('s', 1, 'c'), 'aces', {}), headSubject(history('t', 1, 'b'), 'aces', {}),
    headSubject(history('s', 2, 'b'), 'aces', {}), headSubject(history('s', 1, 'b'), 'yards', {}), headSubject(history('s', 1, 'b'), 'aces', { tag: 'GER' })])
    assert.equal(retain(kept, client, subject, undefined, null), null, `${subject}: another subject never shows the kept reply`);
  assert.equal(retain(kept, other, a, undefined, null), null, 'nor another client');
  assert.equal(retain(kept, client, null, undefined, null), null, 'nor no subject');
  assert.equal(headSubject(undefined, 'aces', {}), null);
  assert.equal(headSubject(history('s', 1, 'b'), 'aces', null), null);
});

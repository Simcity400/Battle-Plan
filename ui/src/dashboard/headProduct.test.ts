import assert from 'node:assert/strict';
import test from 'node:test';
import { headRead, headSubject, momentHour, retain, headKey } from './headProduct.ts';
import type { BoundTrack, WidgetSpec } from './model';
import { observeOnScreen, screenHolder } from './follow/onScreen.ts';

const track = { id: 'main', axisOffset: 10, branch: { id: 'b', firstHour: 100, lastHour: 500 } } as unknown as BoundTrack;
const widget = (time: WidgetSpec['time']) => ({ kind: 'naval-yards', trackIds: ['main'], time }) as Pick<WidgetSpec, 'time' | 'kind' | 'trackIds'>;

test('content scrolled inside a visible widget, or replaced inside a pop-out, is watched by the container that outlives it', t => {
  let watched: Element | undefined;
  const placement = { closest: () => null } as unknown as Element;
  const inside = (container: string) => ({ closest: (selectors: string) => selectors.split(',').map(selector => selector.trim()).includes(container) ? placement : null }) as Element;
  const scope = globalThis as unknown as Record<string, unknown>;
  const before = [scope.IntersectionObserver, scope.getComputedStyle];
  t.after(() => { [scope.IntersectionObserver, scope.getComputedStyle] = before; });
  scope.IntersectionObserver = class { observe(e: Element) { watched = e; } disconnect() {} };
  scope.getComputedStyle = () => ({ display: 'block' });
  for (const container of ['article.dash-widget', '[popover]']) {
    watched = undefined;
    const stop = observeOnScreen(inside(container), () => {});
    assert.equal(watched, placement, `content inside ${container} is watched by it`);
    assert.equal(screenHolder(inside(container)), placement, `a head keeps ${container} as its target, not content that may be replaced`);
    stop();
  }
  const loose = inside('nothing');
  assert.equal(screenHolder(loose), loose, 'content in no container is its own target');
});

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

test('moving a dated pin backward clears the previous reply until the explicit target lands', () => {
  const client = {};
  const later = retain(null, client, 'projects', 'later header', 50, { exact: 50 });
  assert.equal(retain(later, client, 'projects', 'later header', 50, { exact: 12 }), null);
  assert.equal(retain(later, client, 'projects', undefined, null, { exact: 12 }), null);
  assert.equal(retain(later, client, 'projects', 'earlier header', 12, { exact: 12 })?.data, 'earlier header');
});

test('review 8 4: a detail timed by its own hour owns its timing, and the card keeps one placement through its own pin', () => {
  assert.notEqual(headKey('card', 0, 40), headKey('card', 0, null), 'a hovered detail never writes its pin into the followed parent');
  assert.equal(headKey('card', 0, null), headKey('card', 0, null));
});

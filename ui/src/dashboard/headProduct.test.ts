import assert from 'node:assert/strict';
import test from 'node:test';
import { headRead, headSource, headSubject, heldPanel, momentHour, retain, watchBoundary, whenDue, withHeld } from './headProduct.ts';
import { PlaybackController } from './playback.ts';
import type { FrameEngine, EngineFrame } from '../engine/frame';
import { WidgetFrames } from './widgetClock.ts';
import type { HeadProduct } from './headProduct';
import type { BoundTrack, WidgetSpec } from './model';
import type { FollowEntry } from './follow/store';
import { observeOnScreen, screenHolder } from './follow/useFollow.ts';

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

test('a followed panel is asked by the fallback’s clock until its first push lands, and read only from pushes after, whatever a seek empties', () => {
  assert.equal(headSource(true, 'followed', false), 'fallback', 'a follow whose first push has not landed asks the hour sought');
  assert.equal(headSource(true, 'followed', true), 'pushed', 'once a push has landed, a seek that empties the reading keeps the pushed source and its surface');
  assert.equal(headSource(true, 'pending', false), 'fallback');
  assert.equal(headSource(true, 'unavailable', true), 'fallback');
  assert.equal(headSource(true, 'off', false), 'asked', 'not watched yet, or with no element, it is asked as rendered');
  assert.equal(headSource(true, 'off', false, true), 'idle', 'following but seen off screen, nothing is read for it, whatever renders around it');
  assert.equal(headSource(false, 'off', false, true), 'asked', 'not following, its own hour is asked wherever it is');
  assert.equal(headSource(false, 'followed', true), 'asked');
});

test('a followed head reads only the panel of the hour shown, and holds the last landed one apart, only while that hour is read', () => {
  const landed = { panel: { data: 'hour 23', refused: null }, values: new Map([['GER:total', 3]]), hour: 23, through: 23 };
  const entry = { id: 1, status: 'followed', answer: undefined, panel: landed.panel, values: landed.values, landed, hour: 23, through: 23, version: 1 } as FollowEntry;
  assert.equal(heldPanel(entry), undefined, 'the panel of the hour shown is the reading, nothing is held');
  Object.assign(entry, { panel: undefined, values: undefined });
  assert.equal(heldPanel(entry), landed, 'a step that empties the reading holds the last panel with its values and hours');
  Object.assign(entry, { landed: null });
  assert.equal(heldPanel(entry), undefined, 'a backward seek holds nothing');
  assert.equal(headSource(true, 'followed', entry.landed !== undefined), 'pushed', 'and the follow keeps its pushed source');
  assert.equal(heldPanel(null), undefined);
  const live = { read: () => ({ get: () => 3, missing: () => '' }), subscribe: () => () => {} };
  const pending = { data: undefined, at: null, atNow: () => null, shownAt: () => null, held: { data: 'hour 23', hour: 23, through: 23, live } } as unknown as HeadProduct<string>;
  const drawn = withHeld(pending);
  assert.deepEqual([pending.data, drawn.data, drawn.at, drawn.atNow(), drawn.shownAt(), drawn.live], [undefined, 'hour 23', 23, 23, 23, live],
    'only a drawn host, which dims it, is handed the held reading, dated at its own hours');
  const current = { ...pending, data: 'hour 24' } as HeadProduct<string>;
  assert.equal(withHeld(current), current, 'once the hour shown lands it is read');
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

test('a future fallback reply wakes on its first eligible clock frame', () => {
  let hour = 10, wakes = 0;
  const frames = new Set<() => void>();
  const stop = whenDue(14, () => hour, frame => { frames.add(frame); return () => frames.delete(frame); }, () => wakes++);
  for (hour of [11, 13, 14, 16]) {
    frames.forEach(frame => frame());
    assert.equal(wakes, hour >= 14 ? 1 : 0);
  }
  assert.equal(frames.size, 0);
  stop();
});

test('continuous playback across a track’s end tells a following widget once, when its hour leaves the track, and not on frames within it', t => {
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  const tasks = new Map<string, (frame: EngineFrame) => boolean | void>();
  const engine = { every: (phase: string, run: (frame: EngineFrame) => boolean | void) => {
    tasks.set(phase, run);
    return { wake() {}, stop() { tasks.delete(phase); } };
  } } as unknown as FrameEngine;
  const tick = (ms: number) => {
    now += ms;
    for (const phase of ['clock', 'paint']) tasks.get(phase)?.({ now, dt: ms, index: 0, phase } as EngineFrame);
  };
  const page = new PlaybackController(engine);
  t.after(() => page.stop());
  const ending = { id: 'side', axisOffset: 0, branch: { id: 's', firstHour: 0, lastHour: 100 } } as unknown as BoundTrack;
  const read = headRead({ kind: 'aces', trackIds: ['side'], time: { mode: 'follow' } }, ending, [ending], null);
  const widgets = [new WidgetFrames(page, 'aces'), new WidgetFrames(page, 'aces-2')];
  const told: [number, boolean][] = [];
  const stops = widgets.map((widget, index) => watchBoundary(widget.clock, () => read.hourAt(widget.clock.getHour()) === null, () => told.push([index, read.hourAt(page.getHour()) === null])));
  const starting = { id: 'late', axisOffset: 0, branch: { id: 'l', firstHour: 100, lastHour: 500 } } as unknown as BoundTrack;
  const late = headRead({ kind: 'doctrine-mastery', trackIds: ['late'], time: { mode: 'follow' } }, starting, [starting], null);
  const entered: boolean[] = [];
  const stopLate = watchBoundary(page, () => late.hourAt(page.getHour()) === null, () => entered.push(late.hourAt(page.getHour()) === null));
  page.setBounds({ firstHour: 0, lastHour: 500 });
  page.seek(99);
  page.setSpeed(1440);
  page.setPlaying(true);
  tick(0.5);
  assert.deepEqual([page.getHour(), told], [99, []], 'a frame within the track tells nothing');
  tick(16);
  assert.ok(page.getHour() > 100, `the clock crossed the track’s end in one frame, to ${page.getHour()}`);
  assert.deepEqual(told, [[0, true], [1, true]], 'each widget is told once that its hour has left the track');
  assert.deepEqual(entered, [false], 'a widget on a track that starts later is told once, while playing, that its hour has entered the track');
  tick(16); tick(16);
  assert.equal(told.length, 2, 'later frames outside the track tell nothing');
  page.seek(50);
  assert.deepEqual(told.slice(2), [[0, false], [1, false]], 'a seek back inside tells each once');
  for (const stop of [...stops, stopLate]) stop();
  page.seek(300);
  assert.equal(told.length, 4, 'a stopped watch hears nothing');
});

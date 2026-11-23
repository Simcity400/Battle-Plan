import assert from 'node:assert/strict';
import test from 'node:test';
import { headKey, headRead, headSubject, momentHour, retain } from './headProduct.ts';
import { PlaybackController } from './playback.ts';
import type { FrameEngine, EngineFrame } from '../engine/frame';
import { WidgetFrames } from './widgetClock.ts';
import { followSpan, watchBoundary } from './widgetTime.ts';
import { at, FOLLOW, SPEC, type TrackHour, axisAt } from './regionTime.ts';
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
  const following = headRead({ widget: widget({ mode: 'follow' }) as WidgetSpec, action: SPEC }, track, [track]);
  assert.equal(following.follows, true);
  assert.equal(following.hourAt(axisAt(200.7)), 210, 'its fallback asks the clock’s hour on the track');
  const hovered = headRead({ widget: widget({ mode: 'follow' }) as WidgetSpec, action: at(321 as TrackHour) }, track, [track]);
  assert.deepEqual([hovered.follows, hovered.hourAt(axisAt(200))], [false, 321], 'a hovered hour is asked whatever the head does');
  const pinned = headRead({ widget: widget({ mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 50 } } }) as WidgetSpec, action: SPEC }, track, [track]);
  assert.deepEqual([pinned.follows, pinned.hourAt(axisAt(200)), pinned.hourAt(axisAt(400))], [false, 150, 150], 'a pinned hour never moves with the clock');
  const range = headRead({ widget: widget({ mode: 'range', firstOffset: 0, lastOffset: 80 }) as WidgetSpec, action: SPEC }, track, [track]);
  assert.deepEqual([range.follows, range.hourAt(axisAt(0))], [false, 180], 'a range is read at its end');
});

test('an hour outside the recorded branch, or no track, asks nothing', () => {
  const following = headRead({ widget: widget({ mode: 'follow' }) as WidgetSpec, action: SPEC }, track, [track]);
  assert.equal(following.hourAt(axisAt(50)), null);
  assert.equal(following.hourAt(axisAt(491)), null);
  assert.equal(headRead({ widget: widget({ mode: 'follow' }) as WidgetSpec, action: SPEC }, undefined, []).hourAt(axisAt(200)), null);
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
  assert.notEqual(headKey('card', 0, { action: at(40) }), headKey('card', 0, { action: SPEC }), 'a hovered detail never writes its pin into the followed parent');
  assert.notEqual(headKey('card', 0, { action: FOLLOW }), headKey('card', 0, { action: SPEC }), 'a drill following playback under a pinned card has its own placement');
  assert.equal(headKey('card', 0, { action: at(40) }), headKey('card', 0, { action: at(52) }), 'a detail keeps one placement as its hour moves');
  assert.equal(headKey('card', 0, { action: SPEC }), headKey('card', 0, { action: SPEC }));
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
  const read = headRead({ widget: { kind: 'aces', trackIds: ['side'], time: { mode: 'follow' } } as WidgetSpec, action: SPEC }, ending, [ending]);
  const widgets = [new WidgetFrames(page, 'aces'), new WidgetFrames(page, 'aces-2')];
  const told: [number, boolean][] = [];
  const stops = widgets.map((widget, index) => watchBoundary(widget.clock, followSpan(ending), () => told.push([index, read.hourAt(axisAt(page.getHour())) === null])));
  const starting = { id: 'late', axisOffset: 0, branch: { id: 'l', firstHour: 100, lastHour: 500 } } as unknown as BoundTrack;
  const late = headRead({ widget: { kind: 'doctrine-mastery', trackIds: ['late'], time: { mode: 'follow' } } as WidgetSpec, action: SPEC }, starting, [starting]);
  const entered: boolean[] = [];
  const stopLate = watchBoundary(page, followSpan(starting), () => entered.push(late.hourAt(axisAt(page.getHour())) === null));
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

test('a frame re-reads only the boundary watches whose span it crossed, however many are registered', t => {
  const tasks = new Map<string, (frame: EngineFrame) => boolean | void>();
  const engine = { every: (phase: string, run: (frame: EngineFrame) => boolean | void) => {
    tasks.set(phase, run);
    return { wake() {}, stop() { tasks.delete(phase); } };
  } } as unknown as FrameEngine;
  const page = new PlaybackController(engine);
  t.after(() => page.stop());
  page.setBounds({ firstHour: 0, lastHour: 100_000 });
  page.seek(0);
  let reads = 0;
  const told: number[] = [];
  const stops = Array.from({ length: 1500 }, (_, index) => watchBoundary(page, axis => {
    reads++;
    const width = 10 * (index + 1), low = Math.floor(axis / width) * width;
    return [low, low + width];
  }, () => told.push(index)));
  assert.equal(reads, 1500, 'each watch reads its span once when registered');
  for (const hour of [1, 5, 9]) page.seek(hour);
  assert.deepEqual([reads, told], [1500, []], 'moves within every span read nothing');
  page.seek(15);
  assert.deepEqual([reads, told], [1501, [0]], 'crossing one boundary re-reads one watch');
  page.seek(25);
  assert.deepEqual([reads, told.sort((a, b) => a - b)], [1503, [0, 0, 1]], 'crossing two boundaries re-reads two watches');
  for (const stop of stops) stop();
  page.seek(10_000);
  assert.equal(reads, 1503, 'released watches are never read again');
});

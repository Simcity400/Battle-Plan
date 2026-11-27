import test from 'node:test';
import assert from 'node:assert/strict';
import { PlaybackController, MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED } from './playback.ts';
import { WidgetFrames, pageClock } from './widgetClock.ts';
import { pipelineFor, type PipelineHost } from './follow/pipeline.ts';
import type { DashboardClient, DashboardSource } from './client';
import { FrameEngine } from '../engine/frame.ts';

test('fifty scrub moves hold the clock and its pictures until one release seek', () => {
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 10, lastHour: 100 });
  clock.seek(80);
  const before = clock.getSnapshot();
  const calls = { published: 0, anchored: 0, discarded: 0, changed: 0, framed: 0 };
  clock.onPublish(() => calls.published++);
  clock.onAnchor(() => calls.anchored++);
  clock.onDiscard(() => calls.discarded++);
  clock.subscribe(() => calls.changed++);
  clock.subscribeFrame(() => calls.framed++);
  const positions: (number | null)[] = [];
  const stop = clock.subscribeScrub(() => positions.push(clock.getScrubPosition()));
  for (let hour = 60; hour > 10; hour--) clock.scrub(hour);
  assert.equal(clock.getSnapshot(), before);
  assert.deepEqual(calls, { published: 0, anchored: 0, discarded: 0, changed: 0, framed: 0 });
  assert.equal(positions.length, 50);
  clock.seek(clock.getScrubPosition()!);
  clock.endScrub();
  assert.equal(clock.getSnapshot().seekSequence - before.seekSequence, 1);
  assert.equal(clock.getHour(), 11);
  assert.equal(positions.at(-1), null);
  stop();
  clock.scrub(200);
  assert.equal(clock.getScrubPosition(), 100);
  clock.scrub(-1);
  assert.equal(clock.getScrubPosition(), 10);
  clock.scrub(NaN);
  assert.equal(clock.getScrubPosition(), 10);
  assert.equal(positions.length, 51);
  clock.setBounds(null);
  assert.equal(clock.getScrubPosition(), null);
});

test('a playing scrub pauses once without seeking and leaves independent clocks and export alone', t => {
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  const frames = new FrameEngine({ request() {} });
  const clock = new PlaybackController(frames);
  const other = new PlaybackController(frames, 'battle-viewer');
  for (const controller of [clock, other]) {
    controller.setBounds({ firstHour: 10, lastHour: 100 });
    controller.setPlaying(true);
    t.after(() => controller.stop());
  }
  const seek = clock.getSnapshot().seekSequence;
  let anchors = 0;
  clock.onAnchor(() => anchors++);
  now += 500;
  clock.scrub(30);
  const paused = clock.getSnapshot();
  assert.equal(paused.position, 10.5);
  for (let hour = 31; hour <= 80; hour++) clock.scrub(hour);
  assert.equal(clock.getSnapshot(), paused);
  assert.equal(anchors, 1);
  assert.equal(paused.seekSequence, seek);
  assert.equal(other.getSnapshot().playing, true);
  assert.equal(other.getScrubPosition(), null);
  clock.seek(80);
  clock.endScrub();
  assert.equal(clock.getSnapshot().seekSequence, seek + 1);
  assert.equal(clock.getSnapshot().playing, false);
  other.stepExport(40.25, 24, false);
  assert.equal(other.getSnapshot().position, 40.25);
  assert.equal(other.getSnapshot().hoursPerSecond, 24);
  assert.equal(other.getSnapshot().playing, false);
  assert.equal(other.getSnapshot().seekSequence, seek);
});

test('visible widgets on one page share native subscriptions through their individual frame signals', () => {
  const clock = new PlaybackController();
  const client = { available: true, product: () => new Promise(() => {}) } as unknown as DashboardClient;
  const source = { id: 'recording', revision: '1' } as DashboardSource;
  const host = (): PipelineHost => ({ lanes: [{ onmessage: null, postMessage() {} }], open: () => new Promise(() => {}), channel: () => new MessageChannel(),
    frameRate: () => 60, rate: () => 1 });
  const a = new WidgetFrames(clock, 'research'), b = new WidgetFrames(clock, 'active-research');
  const pipeline = pipelineFor(pageClock(a.clock), client, host);
  try {
    assert.equal(pipelineFor(pageClock(b.clock), client, host), pipeline, 'both frame signals reach the page clock\'s one pipeline');
    const query = { product: 'series-state' };
    const one = pipeline.acquire('research|0', { offset: 0, pin: null }), two = pipeline.acquire('active-research|0', { offset: 0, pin: null });
    const shared = pipeline.subscribe(one, {}, source, '1', 0, query, 'followed');
    assert.equal(pipeline.subscribe(two, {}, source, '1', 0, query, 'followed'), shared);
    assert.notEqual(pipeline.subscribe(two, {}, source, '1', 24, query, 'followed'), shared, 'another clock offset is another subscription');
  } finally { pipeline.close(); }
});

test('custom playback rates accept fractions without seeking or starting playback', () => {
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 100, lastHour: 200 });
  clock.seek(150);
  const sequence = clock.getSnapshot().sequence;
  for (const rate of [MIN_PLAYBACK_SPEED, 2.5, 7, MAX_PLAYBACK_SPEED]) {
    clock.setSpeed(rate);
    const snapshot = clock.getSnapshot();
    assert.equal(snapshot.hoursPerSecond, rate);
    assert.equal(snapshot.hour, 150);
    assert.equal(snapshot.sequence, sequence);
    assert.equal(snapshot.playing, false);
  }
});

test('discarded pictures leave before a rewound clock is visible to a frame listener', () => {
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: 100 });
  clock.seek(80);
  let painted: number | null = 80;
  clock.onDiscard(() => { assert.equal(clock.getHour(), 80); painted = null; });
  clock.subscribeFrame(() => assert.ok(painted === null || painted <= clock.getHour()));
  clock.seek(10);
  assert.equal(painted, null);
});

test('invalid custom rates leave the current speed unchanged', () => {
  const clock = new PlaybackController();
  clock.setSpeed(2.5);
  for (const rate of [0, -1, NaN, Infinity, -Infinity, MIN_PLAYBACK_SPEED / 2, MAX_PLAYBACK_SPEED + 1]) {
    clock.setSpeed(rate);
    assert.equal(clock.getSnapshot().hoursPerSecond, 2.5);
  }
});



test('a subscriber that stops the clock on a speed change does not count the same time twice', t => {
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  const clock = new PlaybackController();
  t.after(() => clock.stop());
  clock.setBounds({ firstHour: 0, lastHour: 100_000 });
  clock.setSpeed(168);
  clock.setPlaying(true);
  now += 40;
  let stopped = false;
  clock.subscribe(() => { if (!stopped && clock.getSnapshot().hoursPerSecond === 720) { stopped = true; clock.stop(); } });
  clock.setSpeed(720);
  const { position, playing } = clock.getSnapshot();
  assert.equal(playing, false);
  assert.ok(Math.abs(position - 0.04 * 168) < 1e-9, `stopped at ${position}`);
});

test('pausing and changing speed keep the time played since the last tick', t => {
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  const clock = new PlaybackController();
  t.after(() => clock.stop());
  clock.setBounds({ firstHour: 0, lastHour: 100_000 });
  clock.setSpeed(720);
  clock.setPlaying(true);
  now += 20;
  const shown = clock.positionAt();
  clock.setSpeed(168);
  assert.ok(clock.getSnapshot().position >= shown, 'changing speed never steps back');
  const beforePause = clock.positionAt();
  clock.setPlaying(false);
  assert.ok(clock.getSnapshot().position >= beforePause, 'pausing never steps back');
  assert.equal(clock.positionAt(performance.now() + 1000), clock.getSnapshot().position);
});




test('seek sequence survives coalesced playback updates but not pauses, speed changes or the branch end', t => {
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  const clock = new PlaybackController();
  t.after(() => clock.stop());
  clock.setBounds({ firstHour: 0, lastHour: 1000 });
  const initial = clock.getSnapshot().seekSequence;
  clock.setSpeed(720);
  clock.setPlaying(true);
  now += 100;
  clock.setPlaying(false);
  assert.equal(clock.getSnapshot().hour, 72);
  assert.equal(clock.getSnapshot().seekSequence, initial);
  clock.seek(200);
  clock.setPlaying(true);
  now += 100;
  clock.setSpeed(168);
  assert.equal(clock.getSnapshot().hour, 272);
  assert.equal(clock.getSnapshot().seekSequence, initial + 1);
  clock.seek(272);
  assert.equal(clock.getSnapshot().seekSequence, initial + 2);
  clock.setPlaying(true);
  now += 10_000;
  clock.setPlaying(false);
  assert.equal(clock.getSnapshot().hour, 1000);
  assert.equal(clock.getSnapshot().seekSequence, initial + 2);
});


test('next hour banks play time since the last displayed frame', t => {
  let now = 0;
  t.mock.method(performance, 'now', () => now);
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: 100 });
  clock.setSpeed(24); clock.setPlaying(true);
  now = 625;
  clock.step(1);
  assert.equal(clock.getHour(), 16);
  assert.equal(clock.getSnapshot().playing, false);
});

test('forward steps keep animation history and backward steps reset it', () => {
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 10, lastHour: 20 });
  const initial = clock.getSnapshot().seekSequence;
  clock.step(1);
  assert.equal(clock.getSnapshot().hour, 11);
  assert.equal(clock.getSnapshot().seekSequence, initial);
  clock.step(-1);
  assert.equal(clock.getSnapshot().hour, 10);
  assert.equal(clock.getSnapshot().seekSequence, initial + 1);
  clock.seek(11);
  assert.equal(clock.getSnapshot().seekSequence, initial + 2);
});

test('the 1440 h/s preset banks fractional positions and stops exactly at the branch end', t => {
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  const clock = new PlaybackController();
  t.after(() => clock.stop());
  clock.setBounds({ firstHour: 100, lastHour: 1000 });
  clock.setSpeed(1440);
  clock.setPlaying(true);
  now += 4.5;
  clock.setPlaying(false);
  const paused = clock.getSnapshot();
  assert.equal(paused.hour, 106);
  assert.ok(Math.abs(paused.fraction - 0.48) < 1e-10);
  clock.setPlaying(true);
  now += 1000;
  clock.setPlaying(false);
  assert.equal(clock.getSnapshot().position, 1000);
  assert.equal(clock.getSnapshot().fraction, 0);
  assert.equal(clock.getSnapshot().playing, false);
});


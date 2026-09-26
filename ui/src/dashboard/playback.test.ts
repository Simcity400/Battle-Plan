import test from 'node:test';
import assert from 'node:assert/strict';
import { PlaybackController, MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED, PLAYBACK_SPEEDS, speedLabel, speedParts, validPlaybackSpeed } from './playback.ts';

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

test('invalid custom rates leave the current speed unchanged', () => {
  const clock = new PlaybackController();
  clock.setSpeed(2.5);
  for (const rate of [0, -1, NaN, Infinity, -Infinity, MIN_PLAYBACK_SPEED / 2, MAX_PLAYBACK_SPEED + 1]) {
    clock.setSpeed(rate);
    assert.equal(clock.getSnapshot().hoursPerSecond, 2.5);
  }
});

test('rate labels use whole weeks or days where they fit, and round other rates', () => {
  assert.equal(speedLabel(0.25), '0.25 h/s');
  assert.equal(speedLabel(4), '4 h/s');
  assert.equal(speedLabel(12), '12 h/s');
  assert.equal(speedLabel(24), '1 day/s');
  assert.equal(speedLabel(48), '2 days/s');
  assert.equal(speedLabel(36), '36 h/s');
  assert.equal(speedLabel(168), '1 week/s');
  assert.equal(speedLabel(720), '30 days/s');
  assert.equal(speedLabel(33.333333333), '33.33 h/s');
  assert.deepEqual(speedParts(168), ['1', 'week/s']);
  assert.ok(PLAYBACK_SPEEDS.every(validPlaybackSpeed));
  assert.equal(PLAYBACK_SPEEDS.at(-1), MAX_PLAYBACK_SPEED);
});

test('the position carries forward between ticks only while playing and stops at the branch end', t => {
  const clock = new PlaybackController();
  t.after(() => clock.stop());
  clock.setBounds({ firstHour: 0, lastHour: 1000 });
  clock.seek(100);
  assert.equal(clock.positionAt(performance.now() + 5000), 100);
  clock.setSpeed(720);
  const before = performance.now();
  clock.setPlaying(true);
  // The clock starts counting no earlier than before, so half a second adds at most 360 hours.
  const ahead = clock.positionAt(before + 500);
  assert.ok(ahead >= 100 && ahead <= 100 + 0.5 * 720);
  assert.equal(clock.positionAt(before + 60_000), 1000);
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
  const clock = new PlaybackController();
  t.after(() => clock.stop());
  clock.setBounds({ firstHour: 0, lastHour: 100_000 });
  clock.setSpeed(720);
  clock.setPlaying(true);
  const busy = performance.now() + 20;
  while (performance.now() < busy) { /* let time pass without a tick */ }
  const shown = clock.positionAt();
  clock.setSpeed(168);
  assert.ok(clock.getSnapshot().position >= shown, 'changing speed never steps back');
  const beforePause = clock.positionAt();
  clock.setPlaying(false);
  assert.ok(clock.getSnapshot().position >= beforePause, 'pausing never steps back');
  assert.equal(clock.positionAt(performance.now() + 1000), clock.getSnapshot().position);
});

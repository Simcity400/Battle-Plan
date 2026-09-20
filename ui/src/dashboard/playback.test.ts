import test from 'node:test';
import assert from 'node:assert/strict';
import { PlaybackController, MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED } from './playback.ts';

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

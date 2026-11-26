import assert from 'node:assert/strict';
import test from 'node:test';
import { PlaybackController } from '../dashboard/playback.ts';
import { WidgetFrames } from '../dashboard/widgetClock.ts';
import { BattleClock, PLAY_SECONDS, playRate, sharedPause } from './battleClock.ts';

function host() {
  let now = 0;
  const frames: (() => void)[] = [];
  return {
    host: { now: () => now, next: (run: () => void) => { frames.push(run); return () => { frames.splice(frames.indexOf(run), 1); }; } },
    advance(ms: number) { now += ms; const due = frames.splice(0); due.forEach(run => run()); },
    pending: () => frames.length,
  };
}

test('the hours a battle window has shown are not owed again, until the clock is moved by hand', () => {
  const fake = host();
  const clock = new BattleClock({ first: 100, last: 200 }, 100, fake.host);
  clock.play();
  fake.advance(5000);
  clock.pause();
  assert.equal(clock.resumeFrom, 100, 'nothing shown yet: every hour from where play started is owed');
  clock.acknowledge(103, clock.sequence);
  assert.equal(clock.resumeFrom, 103, 'a window opened again resumes at the last hour shown');
  clock.acknowledge(150, clock.sequence);
  assert.equal(clock.seenThrough, 103, 'an hour the clock has not reached is never acknowledged');
  const stale = clock.movedSequence;
  clock.seek(120);
  clock.acknowledge(103, stale);
  assert.equal(clock.seenThrough, null, 'a move forgets what was shown, and an answer from before it does not count');
  assert.equal(clock.resumeFrom, 120);
});

test('a battle plays from its first hour to its last in about the same time whatever its length, and stops there', () => {
  assert.equal(playRate({ first: 0, last: 120 }), 120 / PLAY_SECONDS);
  assert.equal(playRate({ first: 0, last: 3 }), 1, 'a short battle does not flash by');
  assert.equal(playRate({ first: 0, last: 10_000_000 }), 24, 'a long battle plays a game day a second, so every hour is drawn');
  const fake = host();
  const clock = new BattleClock({ first: 100, last: 220 }, 100, fake.host);
  const hours: number[] = [];
  clock.subscribe(() => hours.push(clock.getHour()));
  const before = clock.sequence;
  clock.play();
  assert.ok(clock.sequence > before, 'playing is a change the map is told about');
  fake.advance(500);
  assert.equal(clock.positionAt(), 105);
  assert.equal(clock.getHour(), 105);
  fake.advance(PLAY_SECONDS * 1000);
  assert.equal(clock.playing, false);
  assert.equal(clock.positionAt(), 220);
  assert.equal(hours.at(-1), 220);
  assert.equal(fake.pending(), 0, 'a stopped clock asks for no frames');
  const seeks = clock.seekSequence;
  clock.play();
  assert.equal(clock.positionAt(), 100, 'play at the end starts again from the first hour');
  assert.ok(clock.seekSequence > seeks);
  clock.dispose();
  assert.equal(fake.pending(), 0, 'a closed viewer leaves no frame behind');
});

test('seeking clamps to the battle, marks a move back as a seek, and pausing keeps the hour reached', () => {
  const fake = host();
  const clock = new BattleClock({ first: 10, last: 20 }, 50, fake.host);
  assert.equal(clock.positionAt(), 20, 'a start past the battle is its last hour');
  const seeks = clock.seekSequence;
  clock.seek(-5);
  assert.equal(clock.positionAt(), 10);
  assert.equal(clock.seekSequence, seeks + 1);
  const sequence = clock.sequence;
  clock.seek(10);
  assert.equal(clock.sequence, sequence, 'seeking to where it already is changes nothing');
  clock.step(3);
  assert.equal(clock.getHour(), 13);
  assert.equal(clock.seekSequence, seeks + 1, 'a move forward reads on, as the dashboard clock does; only a move back is a seek');
  assert.equal(clock.jumpSequence, 2, 'every move by hand is a jump, forward or back, so the battle window seeks to it instead of walking');
  assert.equal(clock.playedFrom, 13, 'a move says the hour it lands on');
  const moved = clock.movedSequence;
  clock.play();
  assert.equal(clock.playedFrom, 13, 'a play says the hour it starts from, so the battle window walks from there');
  fake.advance(1500);
  clock.pause();
  clock.play();
  fake.advance(100);
  clock.pause();
  assert.equal(clock.playedFrom, 13, 'a pause and a play keep it, so the window still owes every hour played since the last move');
  assert.equal(clock.movedSequence, moved, 'only a move by hand starts the owed hours again');
  assert.ok(clock.sequence > moved);
  assert.equal(clock.positionAt(), 14.6);
  fake.advance(1000);
  assert.equal(clock.positionAt(), 14.6, 'a paused clock stays put');
  assert.equal(clock.jumpSequence, 2, 'playing is not a jump');
});

test('the dashboard resumes only when the last battle viewer that paused it closes, and not after anything else paused it', (t) => {
  const page = new PlaybackController();
  t.after(() => page.stop());
  page.setBounds({ firstHour: 0, lastHour: 1000 });
  const widgets = [new WidgetFrames(page, 'first').clock, new WidgetFrames(page, 'second').clock];
  const open = (clock: ReturnType<typeof sharedPause>) => {
    const paused = clock.isPlaying();
    if (paused) clock.pause();
    return () => { if (paused) clock.play(); };
  };
  const playing = () => page.getSnapshot().playing;
  page.setPlaying(true);
  const closeFirst = open(sharedPause(widgets[0]!));
  const closeSecond = open(sharedPause(widgets[1]!));
  assert.equal(playing(), false);
  closeFirst();
  assert.equal(playing(), false, 'the second widget\'s viewer still holds the page clock');
  closeSecond();
  assert.equal(playing(), true);
  page.setPlaying(false);
  open(sharedPause(widgets[0]!))();
  assert.equal(playing(), false, 'a dashboard paused before any viewer stays paused');
  page.setPlaying(true);
  const leave = open(sharedPause(widgets[0]!));
  page.stop();
  leave();
  assert.equal(playing(), false, 'a viewer closing after the page stopped its clock does not restart it');
  for (const halt of [() => page.setPlaying(false), () => page.seek(10), () => page.step(1), () => page.setBounds({ firstHour: 0, lastHour: 2000 })]) {
    page.setPlaying(true);
    const held = open(sharedPause(widgets[1]!));
    halt();
    held();
    assert.equal(playing(), false, 'a pause, seek, step or new time range while a viewer holds the clock is kept');
  }
  page.setPlaying(true);
  open(sharedPause(widgets[1]!))();
  assert.equal(playing(), true, 'a viewer closed while the page stays resumes it');
});

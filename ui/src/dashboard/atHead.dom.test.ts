import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Shown { open: boolean; month: string; days: number[]; time: string; head: { month: string; day: number; hour: number } }
interface CalendarPlayed { renders: number; changes: number; from: { year: number; month: number }; to: { year: number; month: number }; seen: { head: number; shown: number[] }[] }
type Act = <T>(expression: string) => Promise<T>;

/** The army breakdown's roster and pinned header chip while the head moves: the roster is a text panel the service
 * pushes when it differs, asked of nobody and rendered only when its text differs, and asked at the paced hour where
 * no playhead shows it; the chip renders only when its country's identity changes, its date painted every frame. */
async function breakdownAtTheHead(t: { diagnostic: (message: string) => void }, act: Act) {
  await act('seek(1000)');
  await act('mountBreakdown()');
  await act('pushRoster(1000, ["Infantry division"])');
  t.diagnostic(`roster follow: ${JSON.stringify(await act('breakdown()'))}`);
  assert.equal((await act<{ roster: string }>('breakdown()')).roster, 'Infantry division', 'the pushed text shows');
  await act('rosterPasses()');
  const inside = await act<Played>('play(1440, 40)');
  const quiet = await act<{ renders: number; commits: number }>('rosterPasses()');
  t.diagnostic(`breakdown, ${inside.changes} hour changes with the roster followed: renders ${JSON.stringify(inside.renders)}, requests ${JSON.stringify(inside.requests)}`);
  assert.ok(inside.changes >= 30);
  assert.deepEqual(inside.requests.filter(product => product === 'army-size'), [], 'a followed roster is asked of nobody');
  assert.equal(quiet.commits, 0, `the roster committed no render while its text held (${quiet.renders} passes)`);
  assert.equal(inside.renders.breakdownChip ?? 0, 0, 'the chip did not render as the hour moved');
  assert.equal(inside.tooltips, inside.changes, "the chip's date was painted at every hour change");
  await act('pushRoster(2000, ["Infantry division"])');
  assert.equal((await act<{ commits: number }>('rosterPasses()')).commits, 0, 'the same text again renders nothing');
  await act('pushRoster(2100, ["Armoured division"])');
  assert.equal((await act<{ commits: number }>('rosterPasses()')).commits, 1, 'a text that differs renders once');
  assert.equal((await act<{ roster: string }>('breakdown()')).roster, 'Armoured division');
  await act('seek(4800)');
  const across = await act<Played>('play(1440, 40)');
  t.diagnostic(`breakdown across the rename at 5000 (${across.from} to ${across.to}): renders ${JSON.stringify(across.renders)}`);
  assert.equal(across.renders.breakdownChip, 1, 'one render: the name changed');
  assert.equal((await act<{ chip: string }>('breakdown()')).chip, 'German Reich');
  // No playhead: the roster is asked at the paced hour, as before.
  await act('seek(1000)');
  await act('panelsShown(false)');
  const unfollowed = await act<Played>('play(1440, 40)');
  const asked = unfollowed.requests.filter(product => product === 'template-roster').length;
  t.diagnostic(`breakdown with no playhead: ${asked} roster requests in ${unfollowed.elapsed.toFixed(0)} ms`);
  assert.ok(asked >= 1 && asked <= Math.ceil(unfollowed.elapsed / 250) + 1, `asked at the paced hour: ${asked}`);
  assert.equal((await act<{ roster: string }>('breakdown()')).roster, 'Infantry division');
}

/** The controls' date picker, open while the clock plays: it opens on the head, follows it painting the day without a
 * render, renders once a month, and holds still once the user pages or picks. */
async function calendarFollowsTheHead(t: { diagnostic: (message: string) => void }, act: Act) {
  await act('mountCalendar(2)');
  await act('startCalendar(1440, 60)');
  await act('toggleCalendar()');
  const opened = await act<Shown>('calendar()');
  t.diagnostic(`opened while playing: ${JSON.stringify(opened)}`);
  assert.ok(opened.open);
  assert.notEqual(opened.head.month, 'March 1936', 'the head has left the month playback started in');
  assert.equal(opened.month, opened.head.month, "opens on the head's month");
  assert.deepEqual(opened.days, [opened.head.day], "and highlights the head's day");
  await act('pauseCalendar()');
  await act('toggleCalendar()');
  await act('mountCalendar(4)');
  await act('toggleCalendar()');
  const within = await act<CalendarPlayed>('playCalendar(168, 45)');
  t.diagnostic(`inside one month: ${within.changes} hour changes, ${within.renders} renders, days ${[...new Set(within.seen.map(seen => seen.head))].join(' ')}`);
  assert.ok(within.changes >= 40, `${within.changes} hour changes`);
  assert.deepEqual([within.to.year, within.to.month], [within.from.year, within.from.month], 'stayed in one month');
  assert.equal(within.renders, 0, 'the open calendar did not render as the hour moved');
  assert.ok(within.seen.every(seen => seen.shown.length === 1 && seen.shown[0] === seen.head), "the highlight was the head's day at every hour change");
  assert.ok(new Set(within.seen.map(seen => seen.head)).size >= 3, 'and it moved');
  const across = await act<CalendarPlayed>('playCalendar(1440, 45)');
  const months = (across.to.year * 12 + across.to.month) - (across.from.year * 12 + across.from.month);
  t.diagnostic(`across months: ${months} month changes, ${across.renders} renders`);
  assert.ok(months >= 1);
  assert.equal(across.renders, months, 'one render a month');
  const after = await act<Shown>('calendar()');
  assert.equal(after.month, after.head.month);
  assert.deepEqual(after.days, [after.head.day]);
  await act('nextMonth()');
  const paged = await act<Shown>('calendar()');
  const held = await act<CalendarPlayed>('playCalendar(1440, 40)');
  const still = await act<Shown>('calendar()');
  t.diagnostic(`paged to ${paged.month}; the head moved to ${still.head.month}; renders ${held.renders}`);
  assert.equal(held.renders, 0);
  assert.deepEqual([still.month, still.days, still.time], [paged.month, paged.days, paged.time], 'a paged calendar holds while the head moves');
  assert.notEqual(still.head.month, paged.month);
  await act('pickDay(10)');
  const picked = await act<Shown>('calendar()');
  await act('playCalendar(1440, 40)');
  const draft = await act<Shown>('calendar()');
  assert.deepEqual(picked.days, [10]);
  assert.deepEqual([draft.month, draft.days, draft.time], [picked.month, picked.days, picked.time], 'a picked day holds while the head moves');
  await act('toggleCalendar()');
}

interface Played { tooltips: number; changes: number; from: number; to: number; elapsed: number; renders: Record<string, number>; requests: string[]; dates: number; chip: string; identity: string }

test('nothing that reads the hour at the head renders or asks as it moves; a name that changes renders once; the fallback paces itself', async t => {
  await withNativePage('at-head', '/ui/src/dashboard/atHead.dom.fixture.tsx', page => page.stage('at head', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.atHeadFixture.${expression}`);
    await act('mount(1000)');
    const steady = await act<Played>('play(1440, 40)');
    t.diagnostic(`${steady.changes} hour changes (${steady.from} to ${steady.to}, ${steady.elapsed.toFixed(0)} ms): renders ${JSON.stringify(steady.renders)}, requests ${JSON.stringify(steady.requests)}`);
    assert.ok(steady.changes >= 30, `the hour changed ${steady.changes} times`);
    assert.equal(steady.dates, steady.changes, 'the date is painted at every hour change');
    for (const id of ['strip', 'chip', 'section', 'map', 'identity', 'off']) assert.equal(steady.renders[id] ?? 0, 0, `${id} rendered as the hour moved`);
    assert.deepEqual(steady.requests, [], 'nothing was asked as the hour moved');
    assert.equal(steady.chip, 'Axis');
    // The fallback, and an identity whose timeline is not held, ask by their own paced clock.
    const paced = Math.ceil(steady.elapsed / 250) + 1;
    for (const id of ['fallback', 'unheld']) {
      assert.ok((steady.renders[id] ?? 0) >= 1 && steady.renders[id]! <= paced, `${id} rendered ${steady.renders[id]} times in ${steady.elapsed.toFixed(0)} ms`);
    }
    // Across the rename: the chip, the section and the identity each render once.
    await act('seek(4800)');
    const renamed = await act<Played>('play(1440, 40)');
    t.diagnostic(`across the rename (${renamed.from} to ${renamed.to}): renders ${JSON.stringify(renamed.renders)}`);
    assert.ok(renamed.to > 5000 && renamed.from < 5000);
    for (const id of ['chip', 'section', 'identity']) assert.equal(renamed.renders[id], 1, `${id} renders once for the change`);
    for (const id of ['strip', 'map', 'off']) assert.equal(renamed.renders[id] ?? 0, 0, `${id} rendered`);
    assert.equal(renamed.chip, 'Axis Powers');
    assert.equal(renamed.identity, '5000', 'identities are read at the hour they changed');
    assert.deepEqual(renamed.requests, []);
    // A seek is an anchor: the fallback is told at once, the map's placeholder matches again.
    const seek = await act<Record<string, number>>('seek(200)');
    assert.equal(seek.fallback, 1);
    assert.ok((seek.map ?? 0) >= 1, 'the map renders on an anchor');
    for (const tags of [8, 439]) {
      const micros = await act<number>(`identityCost(${tags}, 20)`);
      t.diagnostic(`identity at the head over ${tags} tags of 20 runs: median ${micros.toFixed(2)} µs a frame`);
    }
    await breakdownAtTheHead(t, act);
    await calendarFollowsTheHead(t, act);
  }, 120_000));
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Shown { open: boolean; month: string; days: number[]; time: string; head: { month: string; day: number; hour: number } }
interface CalendarPlayed { renders: number; changes: number; from: { year: number; month: number }; to: { year: number; month: number }; seen: { head: number; shown: number[] }[] }
type Act = <T>(expression: string) => Promise<T>;

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

async function countriesNamedAtTheHourShown(t: { diagnostic: (message: string) => void }, act: Act) {
  await act('seek(1000)');
  await act('mountCountries(1000)');
  assert.deepEqual(await act('countries()'), { tag: 'Germany', slot: 'Germany', lanes: 'Germany' });
  await act('seek(6000)');
  t.diagnostic(`after the rename at 5000: ${JSON.stringify(await act('countries()'))}`);
  assert.deepEqual(await act('countries()'), { tag: 'German Reich', slot: 'German Reich', lanes: 'German Reich' },
    'renamed at 5000, it is named as then wherever it shows');
  await act('seek(4800)');
  const across = await act<Played>('play(1440, 40)');
  assert.ok(across.from < 5000 && across.to > 5000);
  for (const id of ['tagChip', 'slotChip', 'lanes']) assert.equal(across.renders[id], 1, `${id} renders once for the rename`);
  await act('seek(1000)');
}

interface Played { tooltips: number; changes: number; from: number; to: number; elapsed: number; renders: Record<string, number>; requests: string[]; dates: number; chip: string; identity: string }

test('nothing that reads the hour at the head renders or asks as it moves; a name that changes renders once', async t => {
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
    await act('seek(4800)');
    const renamed = await act<Played>('play(1440, 40)');
    t.diagnostic(`across the rename (${renamed.from} to ${renamed.to}): renders ${JSON.stringify(renamed.renders)}`);
    assert.ok(renamed.to > 5000 && renamed.from < 5000);
    for (const id of ['chip', 'section', 'identity']) assert.equal(renamed.renders[id], 1, `${id} renders once for the change`);
    for (const id of ['strip', 'map', 'off']) assert.equal(renamed.renders[id] ?? 0, 0, `${id} rendered`);
    assert.equal(renamed.chip, 'Axis Powers');
    assert.equal(renamed.identity, '5000', 'identities are read at the hour they changed');
    assert.deepEqual(renamed.requests, []);
    const seek = await act<Record<string, number>>('seek(200)');
    assert.equal(seek.fallback, 1);
    assert.ok((seek.map ?? 0) >= 1, 'the map renders on an anchor');
    await countriesNamedAtTheHourShown(t, act);
    await calendarFollowsTheHead(t, act);
  }, 120_000));
});

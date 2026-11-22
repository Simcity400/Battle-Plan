import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Surface { visible: boolean; opacity: number; busy: boolean; hour: number | null }
interface Shown {
  field: string | null; headedField: string | null; mounts: number; hour: number; strict: number | null; asks: number; listedRead: string; listed: string | null;
  warned: string[]; aces: string | null; shortAces: string | null; goals: string | null; shortGoals: string | null; menus?: string[] | null; raceDescribed: boolean; raceRanking: string; raceLive: string; raceFocused: boolean; focused?: Shown; noDoctrines: string | null; tip: string | null; described: boolean; muted: number;
  panel: Surface; race: Surface; headed: Surface;
}
let mounts = 0, muted = 1, asks = 0;
const lit = (surface: Surface, hour: number) => surface.visible && surface.opacity === 1 && !surface.busy && surface.hour === hour;
const dim = (surface: Surface, hour: number) => surface.visible && surface.opacity === muted && surface.busy && surface.hour === hour;
const surfaces = (state: Shown) => [state.panel, state.race, state.headed];
const kept = (state: Shown) => state.field === 'worker' && state.headedField === 'worker' && state.mounts === mounts;
const at = (state: Shown, hour: number) => kept(state) && state.hour === hour && surfaces(state).every(surface => lit(surface, hour));
const show = (state: Shown | undefined) => JSON.stringify(state);

test('paused, a speed change keeps the exact pictures, a move past the hour they hold dims them on the same surfaces until the worker paints the new hour, nothing of the previous hour is taken or shown as current, and a pause paints the hour it stops on', async t => {
  await withNativePage('paused-clock', '/ui/src/draw/pausedClock.dom.fixture.tsx', page => page.stage('paused clock', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.pausedClock.${expression}`);
    const mounted = await act<Shown>('mount()');
    mounts = mounted.mounts; muted = mounted.muted; asks = mounted.asks;
    assert.ok(muted > 0 && muted < 1, `the production styles give a held picture a muted opacity: ${muted}`);
    assert.ok(at(mounted, 23) && mounted.strict === 23, `the worker paints every picture at the paused hour on the surface the push made: ${show(mounted)}`);
    assert.ok(mounted.aces === 'empty' && mounted.noDoctrines === 'empty' && mounted.tip !== null && mounted.described && mounted.warned.join() === 'read at 23',
      `Aces and a country without doctrines show 23:00, a focused Doctrine mastery row shows its tooltip and description, and Nuclear arsenal reports 23:00's warning: ${show(mounted)}`);
    for (const speed of [168, 1, 1440]) {
      const seen = await act<Shown[]>(`speed(${speed})`);
      const wrong = seen.findIndex(state => !at(state, 23));
      t.diagnostic(`${speed} h/s: ${wrong < 0 ? `shown in all ${seen.length} samples` : `wrong at sample ${wrong}: ${show(seen[wrong])}`}`);
      assert.equal(wrong, -1, `at ${speed} h/s while paused, sample ${wrong} dimmed or lost the exact paused values: ${show(seen[wrong])}`);
    }
    const { seen: midnight, pending } = await act<{ seen: Shown[]; pending: Shown }>('heldSeek(24)');
    assert.equal(pending.listedRead, 'followed',
      `a pop-out widget whose content is removed while 24:00 is read keeps its follow through the observers' callbacks: ${show(pending)}`);
    const held = midnight[0]!;
    assert.ok(kept(held) && held.hour === 24 && surfaces(held).every(surface => dim(surface, 23)),
      `before the worker paints, the previous day's pictures stay visible at the muted opacity and busy: ${show(held)}`);
    assert.ok(kept(pending) && pending.strict === null && dim(pending.headed, 23) && [pending.panel, pending.race].every(surface => dim(surface, 23) || lit(surface, 24)),
      `with the followed reply for 24:00 still pending, the head's data is empty and only the drawn host holds 23:00, dimmed on its surface: ${show(pending)}`);
    assert.ok(pending.aces === 'loading' && pending.noDoctrines === 'loading',
      `while 24:00 is read, Aces shows loading, not a time outside the recording, and Doctrine mastery holding 23:00 shows loading, not 23:00's status: ${show(pending)}`);
    assert.deepEqual(mounted.menus, ['pointer', 'keyboard'], 'a current Doctrine mastery row opens its menu by pointer and by keyboard');
    assert.ok(pending.tip === null && !pending.described, `while Doctrine mastery holds 23:00, its row tooltip and description are withheld: ${show(pending)}`);
    assert.ok(mounted.focused?.raceDescribed && mounted.focused.raceLive.includes('at 23'),
      `a focused Nuclear arsenal race is described by its ranking and announces 23:00: ${show(mounted.focused)}`);
    const speaking = [...midnight, pending].find(state => state.raceLive.includes('at 23') || dim(state.race, 23) && (state.raceDescribed || state.raceRanking !== ''));
    assert.ok(dim(held.race, 23) && speaking === undefined,
      `while Nuclear arsenal holds 23:00, it is described by no ranking and announces nothing of 23:00: ${show(speaking ?? held)}`);
    assert.deepEqual(pending.menus, [], 'while Doctrine mastery holds 23:00, its row opens no menu to jump to 23:00, by pointer or keyboard');
    assert.deepEqual(pending.warned, [], 'while 24:00 is read, Nuclear arsenal reports no warning of 23:00, though its race holds 23:00');
    const framed = midnight.slice(1);
    assert.ok(pending.listed === null && framed.every(state => state.listed !== '23'),
      `from the first frame at 24:00, a widget with no drawn host shows nothing of 23:00: ${show(framed.find(state => state.listed === '23'))}`);
    assert.equal(framed.find(state => state.aces === 'unavailable' || state.tip === mounted.tip), undefined,
      'from the first frame at 24:00, no tooltip of 23:00 is shown, and Aces is never outside the recording');
    const between = midnight.find(state => !kept(state) || !surfaces(state).every(surface => dim(surface, 23) || lit(surface, 24)));
    t.diagnostic(`midnight: ${midnight.length} samples, ${midnight.filter(state => surfaces(state).some(surface => !surface.visible)).length} blank`);
    assert.equal(between, undefined, `every sample shows the previous day dimmed or the next day at full strength, on the surfaces it mounted with: ${show(between)}`);
    const midnightEnd = midnight.at(-1)!;
    assert.ok(at(midnightEnd, 24) && midnightEnd.strict === 24, `the next day is painted at full strength: ${show(midnightEnd)}`);
    assert.ok(midnightEnd.raceDescribed, `the Nuclear arsenal race is described again once 24:00 is painted: ${show(midnightEnd)}`);
    assert.ok(midnightEnd.aces === 'empty' && midnightEnd.tip !== null && midnightEnd.tip !== mounted.tip && midnightEnd.described && midnightEnd.warned.join() === 'read at 24',
      `24:00 lands in every widget, and the row tooltip returns with its day: ${show(midnightEnd)}`);
    const sameDay = await act<Shown[]>('step()');
    const lost = sameDay.find(state => !kept(state) || ![state.panel, state.race].every(surface => lit(surface, 24) || lit(surface, 25)) ||
      !(dim(state.headed, 24) || lit(state.headed, 25)));
    assert.ok(lost === undefined && at(sameDay.at(-1)!, 25),
      `a step within the day keeps the pictures that still hold at full strength, and the followed one dimmed until 25:00 paints: ${show(lost ?? sameDay.at(-1))}`);
    const { taken, pending: unread, done } = await act<{ taken: { type: string; hour: number | null }[]; pending: Shown; done: Shown }>('staleStep()');
    t.diagnostic(`replies of 26:00 delivered after the step to 27:00: ${JSON.stringify(taken)}`);
    assert.ok(taken.some(reply => reply.type === 'structure') && taken.some(reply => reply.type === 'inspection'),
      'a structure and an inspection reply of 26:00 were in flight across the step');
    assert.deepEqual(taken.filter(reply => reply.hour !== null), [], 'after a step to 27:00, the delayed 26:00 structure and inspection replies are not taken as current');
    assert.ok(unread.aces === 'loading' && unread.strict === null && unread.listed === null,
      `while 27:00 is read after the delayed 26:00 replies arrived, no head holds 26:00 as current: ${show(unread)}`);
    assert.ok(at(done, 27) && done.strict === 27, `27:00 is then read and painted at full strength: ${show(done)}`);
    const stoppedAt = 27 + 72;
    const stopped = await act<Shown>('playFor(50)');
    assert.ok(at(stopped, stoppedAt), `50 ms at 1440 h/s pauses on hour ${stoppedAt} and paints it: ${show(stopped)}`);
    const after = await act<Shown[]>('speed(168)');
    assert.ok(after.every(state => at(state, stoppedAt)), `paused again, a speed change neither dims nor hides: ${show(after.find(state => !at(state, stoppedAt)))}`);
    const { seen: rewound, pending: rewinding } = await act<{ seen: Shown[]; pending: Shown }>('heldRewind(30)');
    assert.ok(rewinding.strict === null && rewinding.listed === null && rewinding.headedField === null && !rewinding.headed.visible,
      `a backward seek with the reply for 30:00 still pending leaves the followed head empty, never holding ${stoppedAt}:00: ${show(rewinding)}`);
    assert.equal(rewound.slice(1).find(state => state.headed.visible && state.headed.hour === stoppedAt || state.listed === String(stoppedAt)), undefined,
      `no sample after the backward seek shows ${stoppedAt}:00`);
    const back = rewound.at(-1)!;
    mounts = back.mounts;
    assert.ok(at(back, 30) && back.strict === 30 && back.listed === '30', `30:00 is painted at full strength: ${show(back)}`);
    assert.equal(back.asks, asks, 'every move after the first push was read from pushes: no widget dropped its follow while it had nothing to show');
    const { before: atEnd, seen: stepping, after: pastEnd } = await act<{ before: Shown; seen: Shown[]; after: Shown }>('pastShortEnd()');
    assert.ok(atEnd.shortAces === 'empty' && pastEnd.aces === 'empty' && pastEnd.shortAces === 'unavailable',
      `a step past the end of one track's branch, while another runs on, shows that track's Aces outside the recording, never loading: ${show(pastEnd)}`);
    assert.ok(atEnd.goals === 'content' && atEnd.shortGoals === 'content' && pastEnd.goals === 'content'
      && pastEnd.shortGoals === 'unavailable: This time is outside the recording.',
      `the widget host shows War goals stepped past its track's end outside the recording, while the main track's War goals shows 101: ${show(pastEnd)}`);
    assert.equal(stepping.find(state => state.shortGoals !== 'content' && state.shortGoals !== pastEnd.shortGoals), undefined,
      'stepping War goals past its track\'s end never shows loading');
    for (const [hour, gone, said] of [[120, false, 'GER at 120'], [144, true, 'GER is no longer in the ranking']] as const) {
      const { before, seen, pending, done } = await act<{ before: Shown; seen: Shown[]; pending: Shown; done: Shown }>(`raceHold(${hour}, ${gone})`);
      assert.ok(before.raceFocused && before.raceLive.includes(`at ${before.hour}`), `the race is focused and announces its selected row: ${show(before)}`);
      assert.ok(dim(pending.race, before.hour) && pending.raceFocused, `held, the race keeps focus under the dim: ${show(pending)}`);
      const speaking = [...seen, pending].find(state => state.race.busy && (!state.raceFocused || state.raceDescribed || state.raceLive !== ''));
      assert.equal(speaking, undefined, `while held, the focused race announces nothing: ${show(speaking)}`);
      assert.ok(lit(done.race, hour) && done.raceFocused && done.raceDescribed && done.raceLive === said,
        `once ${hour}:00 is presented, the selected country is announced from the current picture: ${show(done)}`);
    }
  }, 120_000));
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Surface { visible: boolean; opacity: number; busy: boolean; hour: number | null }
interface Shown {
  field: string | null; headedField: string | null; mounts: number; hour: number; strict: number | null; asks: number; listed: string | null; muted: number;
  panel: Surface; race: Surface; headed: Surface;
}
let mounts = 0, muted = 1, asks = 0;
const lit = (surface: Surface, hour: number) => surface.visible && surface.opacity === 1 && !surface.busy && surface.hour === hour;
const dim = (surface: Surface, hour: number) => surface.visible && surface.opacity === muted && surface.busy && surface.hour === hour;
const surfaces = (state: Shown) => [state.panel, state.race, state.headed];
const kept = (state: Shown) => state.field === 'worker' && state.headedField === 'worker' && state.mounts === mounts;
const at = (state: Shown, hour: number) => kept(state) && state.hour === hour && surfaces(state).every(surface => lit(surface, hour));
const show = (state: Shown | undefined) => JSON.stringify(state);

test('paused, a speed change keeps the exact pictures, a move past the hour they hold dims them on the same surfaces until the worker paints the new hour, and a pause paints the hour it stops on', async t => {
  await withNativePage('paused-clock', '/ui/src/draw/pausedClock.dom.fixture.tsx', page => page.stage('paused clock', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.pausedClock.${expression}`);
    const mounted = await act<Shown>('mount()');
    mounts = mounted.mounts; muted = mounted.muted; asks = mounted.asks;
    assert.ok(muted > 0 && muted < 1, `the production styles give a held picture a muted opacity: ${muted}`);
    assert.ok(at(mounted, 23) && mounted.strict === 23, `the worker paints every picture at the paused hour on the surface the push made: ${show(mounted)}`);
    for (const speed of [168, 1, 1440]) {
      const seen = await act<Shown[]>(`speed(${speed})`);
      const wrong = seen.findIndex(state => !at(state, 23));
      t.diagnostic(`${speed} h/s: ${wrong < 0 ? `shown in all ${seen.length} samples` : `wrong at sample ${wrong}: ${show(seen[wrong])}`}`);
      assert.equal(wrong, -1, `at ${speed} h/s while paused, sample ${wrong} dimmed or lost the exact paused values: ${show(seen[wrong])}`);
    }
    const { seen: midnight, pending } = await act<{ seen: Shown[]; pending: Shown }>('heldSeek(24)');
    const held = midnight[0]!;
    assert.ok(kept(held) && held.hour === 24 && surfaces(held).every(surface => dim(surface, 23)),
      `before the worker paints, the previous day's pictures stay visible at the muted opacity and busy: ${show(held)}`);
    assert.ok(kept(pending) && pending.strict === null && dim(pending.headed, 23) && [pending.panel, pending.race].every(surface => dim(surface, 23) || lit(surface, 24)),
      `with the followed reply for 24:00 still pending, the head's data is empty and only the drawn host holds 23:00, dimmed on its surface: ${show(pending)}`);
    const framed = midnight.slice(1);
    assert.ok(pending.listed === null && framed.every(state => state.listed !== '23'),
      `from the first frame at 24:00, a widget with no drawn host shows nothing of 23:00: ${show(framed.find(state => state.listed === '23'))}`);
    const between = midnight.find(state => !kept(state) || !surfaces(state).every(surface => dim(surface, 23) || lit(surface, 24)));
    t.diagnostic(`midnight: ${midnight.length} samples, ${midnight.filter(state => surfaces(state).some(surface => !surface.visible)).length} blank`);
    assert.equal(between, undefined, `every sample shows the previous day dimmed or the next day at full strength, on the surfaces it mounted with: ${show(between)}`);
    assert.ok(at(midnight.at(-1)!, 24) && midnight.at(-1)!.strict === 24, `the next day is painted at full strength: ${show(midnight.at(-1))}`);
    const sameDay = await act<Shown[]>('step()');
    const lost = sameDay.find(state => !kept(state) || ![state.panel, state.race].every(surface => lit(surface, 24) || lit(surface, 25)) ||
      !(dim(state.headed, 24) || lit(state.headed, 25)));
    assert.ok(lost === undefined && at(sameDay.at(-1)!, 25),
      `a step within the day keeps the pictures that still hold at full strength, and the followed one dimmed until 25:00 paints: ${show(lost ?? sameDay.at(-1))}`);
    const stopped = await act<Shown>('playFor(50)');
    assert.ok(at(stopped, 25 + 72), `50 ms at 1440 h/s pauses on hour 97 and paints it: ${show(stopped)}`);
    const after = await act<Shown[]>('speed(168)');
    assert.ok(after.every(state => at(state, 97)), `paused again, a speed change neither dims nor hides: ${show(after.find(state => !at(state, 97)))}`);
    const { seen: rewound, pending: rewinding } = await act<{ seen: Shown[]; pending: Shown }>('heldRewind(30)');
    assert.ok(rewinding.strict === null && rewinding.listed === null && rewinding.headedField === null && !rewinding.headed.visible,
      `a backward seek with the reply for 30:00 still pending leaves the followed head empty, never holding 97:00: ${show(rewinding)}`);
    assert.equal(rewound.slice(1).find(state => state.headed.visible && state.headed.hour === 97 || state.listed === '97'), undefined,
      'no sample after the backward seek shows 97:00');
    const back = rewound.at(-1)!;
    mounts = back.mounts;
    assert.ok(at(back, 30) && back.strict === 30 && back.listed === '30', `30:00 is painted at full strength: ${show(back)}`);
    assert.equal(back.asks, asks, 'every move after the first push was read from pushes: no widget dropped its follow while it had nothing to show');
  }, 120_000));
});

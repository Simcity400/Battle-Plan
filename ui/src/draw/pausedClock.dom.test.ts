import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Shown { field: string | null; panel: boolean; race: boolean; hour: number; panelHour: number | null; raceHour: number | null }
const painted = (state: Shown) => state.field === 'worker' && state.panel && state.race;
const hidden = (state: Shown) => state.field !== 'worker' && !state.panel && !state.race;
const at = (state: Shown, hour: number) => painted(state) && state.hour === hour && state.panelHour === hour && state.raceHour === hour;
const show = (state: Shown | undefined) => JSON.stringify(state);

test('paused, a speed change keeps the exact pictures, a move past the hour they hold hides them until the worker paints the new hour, and a pause paints the hour it stops on', async t => {
  await withNativePage('paused-clock', '/ui/src/draw/pausedClock.dom.fixture.tsx', page => page.stage('paused clock', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.pausedClock.${expression}`);
    const mounted = await act<Shown>('mount()');
    assert.ok(at(mounted, 23), `the worker paints both pictures at the paused hour: ${show(mounted)}`);
    for (const speed of [168, 1, 1440]) {
      const seen = await act<Shown[]>(`speed(${speed})`);
      const blank = seen.findIndex(state => !at(state, 23));
      t.diagnostic(`${speed} h/s: ${blank < 0 ? `shown in all ${seen.length} samples` : `wrong at sample ${blank}: ${show(seen[blank])}`}`);
      assert.equal(blank, -1, `at ${speed} h/s while paused, sample ${blank} lost the exact paused values: ${show(seen[blank])}`);
    }
    const midnight = await act<Shown[]>('seek(24)');
    assert.ok(hidden(midnight[0]!), `before the worker paints, the previous day's pictures are released: ${show(midnight[0])}`);
    assert.ok(midnight.every(state => hidden(state) || at(state, 24)), `no sample shows the previous day at hour 24: ${show(midnight.find(state => !hidden(state) && !at(state, 24)))}`);
    assert.ok(at(midnight.at(-1)!, 24), `the next day is painted: ${show(midnight.at(-1))}`);
    const sameDay = await act<Shown[]>('step()');
    assert.ok(sameDay.every(painted) && at(sameDay.at(-1)!, 25), `a step within the day keeps the pictures that still hold: ${show(sameDay.find(state => !painted(state)) ?? sameDay.at(-1))}`);
    const stopped = await act<Shown>('playFor(50)');
    assert.ok(at(stopped, 25 + 72), `50 ms at 1440 h/s pauses on hour 97 and paints it: ${show(stopped)}`);
    const after = await act<Shown[]>('speed(168)');
    assert.ok(after.every(state => at(state, 97)), `paused again, a speed change hides nothing: ${show(after.find(state => !at(state, 97)))}`);
  }, 120_000));
});

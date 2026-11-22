import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Surface { visible: boolean; dim: boolean; busy: boolean; hour: number | null }
interface Shown { field: string | null; hour: number; panel: Surface; race: Surface }
const lit = (surface: Surface, hour: number) => surface.visible && !surface.dim && !surface.busy && surface.hour === hour;
const dim = (surface: Surface, hour: number) => surface.visible && surface.dim && surface.busy && surface.hour === hour;
const surfaces = (state: Shown) => [state.panel, state.race];
const at = (state: Shown, hour: number) => state.field === 'worker' && state.hour === hour && surfaces(state).every(surface => lit(surface, hour));
const show = (state: Shown | undefined) => JSON.stringify(state);

test('paused, a speed change keeps the exact pictures, a move past the hour they hold dims them until the worker paints the new hour, and a pause paints the hour it stops on', async t => {
  await withNativePage('paused-clock', '/ui/src/draw/pausedClock.dom.fixture.tsx', page => page.stage('paused clock', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.pausedClock.${expression}`);
    const mounted = await act<Shown>('mount()');
    assert.ok(at(mounted, 23), `the worker paints both pictures at the paused hour: ${show(mounted)}`);
    for (const speed of [168, 1, 1440]) {
      const seen = await act<Shown[]>(`speed(${speed})`);
      const wrong = seen.findIndex(state => !at(state, 23));
      t.diagnostic(`${speed} h/s: ${wrong < 0 ? `shown in all ${seen.length} samples` : `wrong at sample ${wrong}: ${show(seen[wrong])}`}`);
      assert.equal(wrong, -1, `at ${speed} h/s while paused, sample ${wrong} dimmed or lost the exact paused values: ${show(seen[wrong])}`);
    }
    const midnight = await act<Shown[]>('seek(24)');
    const held = midnight[0]!;
    assert.ok(held.field === 'worker' && held.hour === 24 && surfaces(held).every(surface => dim(surface, 23)),
      `before the worker paints, the previous day's pictures stay visible, dimmed and busy: ${show(held)}`);
    const between = midnight.find(state => state.field !== 'worker' || !surfaces(state).every(surface => dim(surface, 23) || lit(surface, 24)));
    assert.equal(between, undefined, `every sample shows the previous day dimmed or the next day at full strength: ${show(between)}`);
    assert.ok(at(midnight.at(-1)!, 24), `the next day is painted at full strength: ${show(midnight.at(-1))}`);
    const sameDay = await act<Shown[]>('step()');
    const lost = sameDay.find(state => state.field !== 'worker' || !surfaces(state).every(surface => lit(surface, 24) || lit(surface, 25)));
    assert.ok(lost === undefined && at(sameDay.at(-1)!, 25), `a step within the day keeps the pictures that still hold at full strength: ${show(lost ?? sameDay.at(-1))}`);
    const stopped = await act<Shown>('playFor(50)');
    assert.ok(at(stopped, 25 + 72), `50 ms at 1440 h/s pauses on hour 97 and paints it: ${show(stopped)}`);
    const after = await act<Shown[]>('speed(168)');
    assert.ok(after.every(state => at(state, 97)), `paused again, a speed change neither dims nor hides: ${show(after.find(state => !at(state, 97)))}`);
  }, 120_000));
});

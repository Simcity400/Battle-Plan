import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Shown { field: string | null; panel: boolean; race: boolean; hour: number }
const painted = (state: Shown) => state.field === 'worker' && state.panel && state.race;

test('paused, a speed change keeps the drawn panel and the race picture shown in every frame, and playing and pausing still paints the hour it stops on', async t => {
  await withNativePage('paused-clock', '/ui/src/draw/pausedClock.dom.fixture.tsx', page => page.stage('paused clock', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.pausedClock.${expression}`);
    const mounted = await act<Shown>('mount()');
    assert.ok(painted(mounted), `the worker paints both pictures while paused: ${JSON.stringify(mounted)}`);
    for (const speed of [168, 1, 1440]) {
      const seen = await act<Shown[]>(`speed(${speed})`);
      const blank = seen.findIndex(state => !painted(state));
      t.diagnostic(`${speed} h/s: ${blank < 0 ? 'shown in every frame' : `blank at frame ${blank}: ${JSON.stringify(seen[blank])}`}`);
      assert.equal(blank, -1, `at ${speed} h/s while paused, frame ${blank} hid the exact paused values: ${JSON.stringify(seen[blank])}`);
      assert.ok(seen.every(state => state.hour === mounted.hour), 'the paused hour did not move');
    }
    const stopped = await act<Shown>('playThenPause(20)');
    assert.ok(stopped.hour > mounted.hour && painted(stopped), `after playing, the hour it paused on is painted: ${JSON.stringify(stopped)}`);
    const after = await act<Shown[]>('speed(168)');
    assert.ok(after.every(painted), `paused again, a speed change hides nothing: ${JSON.stringify(after.find(state => !painted(state)))}`);
  }, 120_000));
});

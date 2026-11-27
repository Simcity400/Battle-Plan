import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface State { seeks: number; requests: number; clears: number; pauses: number; position: number; playing: boolean; scrub: number | null;
  handle: number; fill: string; knob: string; date: string; dated: string; expectedDate: string; mapHours: number[]; captured: boolean }

test('a fifty-move timeline drag holds widgets and map, commits once, and a 150 ms dwell adds one seek', async () => {
  await withNativePage('timeline-scrub', '/ui/src/dashboard/controls.dom.fixture.tsx', page => page.stage('timeline scrub', async () => {
    const act = (expression: string) => page.evaluate<State>(`window.timeline.${expression}`);
    await act('mount()');
    await act('press()');
    const held = await page.evaluate<State>('(() => { let state; for (let hour = 60; hour > 10; hour--) state = window.timeline.move(hour); return state; })()');
    assert.deepEqual([held.seeks, held.requests, held.clears, held.position], [0, 0, 0, 80]);
    assert.equal(held.scrub, 11);
    assert.equal(held.handle, 11);
    assert.equal(held.fill, 'scaleX(0.11)');
    assert.equal(held.knob, 'translateX(-89%)');
    assert.equal(held.dated, held.expectedDate);
    assert.equal(held.date, held.expectedDate.replace(' · ', ''));
    assert.deepEqual(held.mapHours, []);
    assert.equal(held.captured, true);
    const release = await act('release()');
    assert.deepEqual([release.seeks, release.requests, release.position, release.scrub, release.playing], [1, 1, 11, null, false]);
    assert.deepEqual(release.mapHours, [11]);
    assert.equal((await act('advance(150)')).seeks, 1, 'release cancels the dwell');

    await act('mount()'); await act('press()'); await act('move(30)');
    assert.equal((await act('advance(149)')).seeks, 0);
    const dwell = await act('advance(1)');
    assert.deepEqual([dwell.seeks, dwell.requests, dwell.scrub], [1, 1, 30]);
    assert.equal((await act('advance(150)')).seeks, 1, 'a stationary dwell commits only once');
    await act('move(40)'); await act('advance(100)'); await act('move(41)');
    assert.equal((await act('advance(50)')).seeks, 1, 'movement restarts the dwell');
    const cancelled = await act('release(true)');
    assert.deepEqual([cancelled.seeks, cancelled.requests, cancelled.position, cancelled.scrub], [2, 2, 41, null]);

    await act('mount()'); await act('press()'); await act('move(30)');
    assert.equal((await act('advance(150)')).seeks, 1);
    await act('move(30)');
    assert.equal((await act('advance(450)')).seeks, 1, 'jitter within the last committed hour does not seek again');
    const settled = await act('release()');
    assert.deepEqual([settled.seeks, settled.requests, settled.position, settled.scrub], [1, 1, 30, null]);

    await act('mount()'); await act('press()'); await act('move(30)'); await act('advance(150)');
    await act('move(40)'); await act('move(30)');
    assert.equal((await act('release()')).seeks, 1, 'returning to the last committed hour does not seek again');

    await act('mount()'); await act('press()'); await act('move(40)'); await act('move(80)');
    assert.equal((await act('release()')).seeks, 1, 'releasing at the starting hour still commits the drag');

    await act('mount(true)'); await act('press()');
    const playing = await page.evaluate<State>('(() => { let state; for (let hour = 60; hour > 10; hour--) state = window.timeline.move(hour); return state; })()');
    assert.deepEqual([playing.seeks, playing.pauses, playing.clears, playing.playing], [0, 1, 0, false]);
    const stopped = await act('release()');
    assert.equal(stopped.seeks, 1);
    assert.equal(stopped.pauses, 1);
    assert.ok(stopped.requests >= 1 && stopped.requests <= 2);
    assert.equal(stopped.playing, false);

    await act('mount()');
    await act('press()');
    assert.equal((await act('release()')).seeks, 0, 'pressing the knob without changing the range does not seek');
    assert.equal((await act('change(25)')).seeks, 1, 'a keyboard change seeks immediately');
    await act('press(35)'); await act('change(35)');
    assert.equal((await act('release()')).seeks, 2, 'a click without movement seeks once');
    await act('press(35)'); await act('move(45)');
    const unmounted = await page.evaluate<{ seeks: number; scrub: number | null }>('window.timeline.unmount()');
    assert.deepEqual(unmounted, { seeks: 2, scrub: null }, 'unmount cancels the drag and its dwell');
  }));
});

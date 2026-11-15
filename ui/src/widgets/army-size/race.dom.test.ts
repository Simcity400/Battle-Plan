import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

const median = (values: number[]) => [...values].sort((a, b) => a - b)[values.length >> 1] ?? NaN;

test('the army race follows the head: values and order painted, never rendered or asked for; it falls back when not followed', async t => {
  await withNativePage('army-race', '/ui/src/widgets/army-size/race.dom.fixture.tsx', page => page.stage('army race', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.armyRace.${expression}`);
    await act('mount(3)');
    const followed = await act<string[]>('sent()');
    assert.equal(followed.filter(line => line.startsWith('follow')).length, 1, `followed on mount: ${followed.join('; ')}`);
    await act('paused(true)');
    await act('deliver(1000, [[1, 300], [2, 200], [3, 100]])');
    assert.deepEqual(await act('drawn()'), ['300', '200', '100']);
    // Counts are read once whatever was started has landed (`quiet`), never after a number of frames.
    await act('quiet()');
    const asked = await act<string[]>('asked()'), rendered = await act<number>('renders()');

    // Thirty hour changes with an answer each: values move, rows swap places; nothing is asked or rendered.
    await act('paused(false)');
    for (let hour = 1; hour <= 30; hour++) {
      const [a, b, c] = hour % 2 ? [100 + hour, 300 + hour, 200 + hour] : [300 + hour, 100 + hour, 200 + hour];
      await act(`play(1, 1)`);
      await act(`deliver(${1000 + hour}, [[1, ${a}], [2, ${b}], [3, ${c}]], 1)`);
    }
    await act('quiet()');
    assert.deepEqual(await act('asked()'), asked, 'no request because the hour changed');
    assert.equal(await act('renders()'), rendered, 'no render because a value or the order changed');
    await act('paused(true)');
    await act('deliver(1031, [[1, 131], [2, 331], [3, 231]])');
    assert.deepEqual(await act('drawn()'), ['331', '231', '131'], 'the newest answer drawn, in its order');

    // The chart paints only on a new answer, at most once a frame: the clock moving alone draws nothing once the
    // race has settled, and three answers in one frame draw once in it.
    await act('paused(false)');
    await act('play(1, 40)');
    await act('draws()');
    await act('play(24, 20)');
    assert.equal(await act('draws()'), 0, 'twenty clock frames without an answer: nothing drawn');
    const burst = await act<number>('burst(1050, 1)');
    assert.ok(burst <= 1, `three answers in one frame: ${burst} draws`);
    await act('play(1, 40)');
    await act('paused(true)');

    // A row comes: one render for the new set of rows.
    await act('deliver(1100, [[1, 131], [2, 331], [3, 231], [4, 500]])');
    assert.ok((await act<number>('renders()')) > rendered, 'a row came: rendered');
    const after = await act<number>('renders()');
    await act('deliver(1101, [[1, 132], [2, 332], [3, 232], [4, 501]])');
    assert.equal(await act('renders()'), after, 'the same rows again: no render');

    // Off screen: not followed, not painted; shown again it follows and catches up.
    await act('offScreen()');
    assert.ok((await act<string[]>('sent()')).some(line => line.startsWith('unfollow')), 'off screen: unfollowed');
    await act('onScreen()');
    await act('deliver(1200, [[1, 1], [2, 2], [3, 3], [4, 4]])');
    assert.deepEqual(await act('drawn()'), ['4', '3', '2'], 'shown again: painted from the newest answer, its top 3');
    await act('unmount()');
    assert.ok((await act<string[]>('sent()')).filter(line => line.startsWith('unfollow')).length >= 2, 'unmounted: unfollowed');

    // Not followed: the race reads its windows as before, one window at a time, and paints the values at the head.
    await act('mount(3, false)');
    const windows = await act<string[]>('asked()');
    assert.ok(windows.some(line => line.startsWith('army-size') && line !== 'army-size null'), `the fallback asks its windows: ${windows.join('; ')}`);
    await act('quiet()');
    const fallbackRenders = await act<number>('renders()');
    await act('play(24, 10)');
    await act('quiet()');
    assert.equal(await act('renders()'), fallbackRenders, 'inside a window, the hour moving renders nothing');
    await act('play(720, 1)');
    assert.ok((await act<string[]>('asked()')).length > windows.length, 'a new window is asked for');
    await act('unmount()');

    // The mix's legend and a pinned breakdown at the head: thirty hours, a day turning, render and ask nothing; their
    // values are painted, the same as a render at that hour shows.
    for (const [view, selector] of [['mix', '.chart-legend b'], ['breakdown', '.drill-part-row b, .drill-total b, .drill-day']] as const) {
      const texts = () => act<string[]>(`texts(${JSON.stringify(selector)})`);
      await act(`mount(3, false, '${view}')`);
      await act('quiet()');
      const before = await texts(), rendered = await act<number>('renders()'), asked = await act<string[]>('asked()');
      await act('play(1, 30)');
      await act('quiet()');
      assert.equal(await act('renders()'), rendered, `${view}: thirty hours render nothing`);
      assert.deepEqual((await act<string[]>('asked()')).slice(asked.length), [], `${view}: thirty hours ask nothing`);
      const painted = await texts();
      assert.notDeepEqual(painted, before, `${view}: the values moved`);
      await act(`mount(3, false, '${view}', 1030)`);
      await act('quiet()');
      assert.deepEqual(painted, await texts(), `${view}: painted as a render at that hour shows it`);
    }

    // Crossing into a window held ahead renders nothing and asks one window, the next; its reply renders.
    for (const view of ['lines', 'mix', 'breakdown']) {
      await act(`mount(3, false, '${view}')`);
      await act('play(0, 4)');
      // To 1469, the last hour before the window starting at 1470.
      for (let step = 0; step < 20; step++) await act('play(23, 1)');
      await act('play(1, 9)');
      await act('quiet()');
      const rendered = await act<number>('renders()'), asked = await act<string[]>('asked()');
      assert.ok(asked.includes('army-size 2') && asked.includes('army-size 3'), `${view}: the next window is held ahead: ${asked.join('; ')}`);
      await act('hold()');
      await act('play(1, 1)');
      await act('quiet()');
      assert.equal(await act('renders()'), rendered, `${view}: crossing into a held window renders nothing`);
      assert.deepEqual((await act<string[]>('asked()')).slice(asked.length).filter(line => line.startsWith('army-size')), ['army-size 4'],
        `${view}: one window asked, the next`);
      await act('release()');
      await act('quiet()');
      assert.ok(await act<number>('renders()') > rendered, `${view}: its reply renders`);
    }
    await act('unmount()');

    // A paint of 8, 100 and 439 rows, one answer a frame: every value and rank moving (`shuffle`), or values drifting a
    // little with the ranks mostly held (`drift`). An answer changing only rows out of view draws nothing.
    const shuffle = (count: number) => (hour: number) => `[${Array.from({ length: count }, (_, at) => `[${at + 1}, ${((at * 37 + hour * 11) % 997) * 1000 + at}]`).join(',')}]`;
    const drift = (count: number) => (hour: number) => `[${Array.from({ length: count }, (_, at) => `[${at + 1}, ${(count - at) * 1000 + ((at + hour) % 5)}]`).join(',')}]`;
    for (const count of [8, 100, 439]) {
      for (const [kind, rows] of [['shuffle', shuffle(count)], ['drift', drift(count)]] as const) {
        await act(`mount(${count})`);
        await act('paused(false)');
        await act(`deliver(2000, ${rows(0)})`);
        await act('passes()');
        for (let hour = 1; hour <= 200; hour++) await act(`deliver(${2000 + hour}, ${rows(hour)}, 1)`);
        const passes = await act<number[]>('passes()'), timed = passes.filter(pass => pass > 0);
        const mean = passes.reduce((sum, pass) => sum + pass, 0) / passes.length;
        t.diagnostic(`race of ${count} rows, ${kind}, an answer a frame: ${passes.length} frames, median ${median(passes).toFixed(3)} ms (of those over 0: ${median(timed).toFixed(3)} ms), mean ${mean.toFixed(3)} ms, worst ${Math.max(...passes).toFixed(3)} ms`);
        // A guard against a regression; the bar is judged on a pinned run with its load beside it.
        assert.ok(median(passes) < (count > 100 ? 2 : 1), `a paint of ${count} rows took ${median(passes).toFixed(3)} ms at the median`);
        if (kind === 'drift' && count >= 100) {
          // Only rows far below the view change: nothing visible moved, nothing is drawn.
          await act('play(1, 40)');
          await act('draws()');
          const rendered = await act<number>('renders()');
          const held = `[${Array.from({ length: count }, (_, at) => `[${at + 1}, ${(count - at) * 1000 + (at >= 40 ? 3 : (at + 200) % 5)}]`).join(',')}]`;
          await act(`deliver(2300, ${held}, 3)`);
          const drew = await act<number>('draws()');
          assert.equal(drew, 0, `an answer whose rows in view did not change draws nothing (renders meanwhile: ${await act<number>('renders()') - rendered})`);
          // At rest, the clock moving with nothing visible changing runs nothing in a frame.
          await act('still()');
          await act('passes()');
          await act('play(1, 30)');
          assert.deepEqual(await act('passes()'), [], `${count} rows: thirty clock frames at rest run nothing`);
        }
        await act('unmount()');
      }
    }
  }, 180_000));
});

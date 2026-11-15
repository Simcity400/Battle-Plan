import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('occupied states at 15 hours a frame: resistance, compliance, the garrison amount and its strength repaint from followed cells with no commit, nothing is asked, the hour shown is reported', async t => {
  await withNativePage('occupied-states', '/ui/src/widgets/occupied-states/occupiedStates.dom.fixture.tsx', page => page.stage('occupied states', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.occupiedStates.${expression}`);
    assert.notEqual(await act('mount()'), null, 'the widget follows its panel');
    const first = await act<(string | null)[][]>('painted()');
    assert.deepEqual(first, [['30%', '3%', '3.50'], ['20%', '3%', '2.50']], 'paused at 30,000: each state\'s resistance, compliance and garrison amount painted, the greater resistance first');
    const played = await act<{ head: number; asked: string[]; commits: number }>('play(600, 15)');
    t.diagnostic(`600 frames to hour ${played.head}: commits ${played.commits}; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.equal(played.commits, 0, 'no React commit as the values moved');
    assert.deepEqual(played.asked, [], 'no request for values at the head');
    const hour = played.head;
    const resistance = (at: number) => `${Math.round((2_000_000 + at * 1_000_000 + (hour % 50) * 10_000) / 100_000)}%`;
    const garrison = (at: number) => ((250_000 + at * 100_000 + (hour % 50) * 1_000) / 100_000).toFixed(2);
    const painted = await act<(string | null)[][]>('painted()');
    assert.deepEqual(painted.map(row => row[0]), [resistance(1), resistance(0)], 'the head\'s values painted');
    assert.deepEqual(painted.map(row => row[2]), [garrison(1), garrison(0)], 'the garrison amount is a followed cell, painted at the head with no commit');
    // The strength was the panel's structure until it was a derived column: it moved only with a push.
    const strength = (at: number) => at === 0 ? 20_000 + (hour % 50) * 1_000 : 60_000 + (hour % 50) * 500;
    const bars = await act<[string | null, boolean][]>('bars()');
    const scale = (transform: string | null) => Number(/^scaleX\(([^)]+)\)$/.exec(transform ?? '')?.[1] ?? NaN);
    // The bar narrows the strength to whole percents, half away from zero, as the game draws it.
    assert.deepEqual(bars.map(([transform, red]) => [Math.round(scale(transform) * 100), red]), [1, 0].map(at => [Math.round(strength(at) / 1_000), strength(at) < 30_000]),
      'the garrison bar and its red follow the strength’s cell at the head with no commit');
    assert.equal(await act('lastShown()'), hour, 'the widget reports the hour of the cells it painted');
    await act('pause()');
    // Run 14: a seek to an hour with no occupied state showed the empty state and never reported the hour sought.
    const sought = await act<{ shown: number | null; empty: boolean }>(`seekEmpty(${hour - 20_000})`);
    assert.deepEqual(sought, { shown: hour - 20_000, empty: true }, 'with no state to draw, the widget still reports the hour it shows');
  }, 180_000));
});

test('occupied states as the real recording pushes them: forty states, a push four times a second at a new hour, their order by resistance moving every hour: one commit a quarter second at most, no name asked per push', async t => {
  await withNativePage('occupied-states-pushed', '/ui/src/widgets/occupied-states/occupiedStates.dom.fixture.tsx', page => page.stage('occupied states pushed', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.occupiedStates.${expression}`);
    assert.notEqual(await act('mount(true)'), null, 'the widget follows its panel');
    const played = await act<{ head: number; elapsed: number; pushes: number; asked: string[]; commits: number; why: Record<string, number> }>('playPushed(600, 15, 15)');
    t.diagnostic(`600 frames to hour ${played.head} in ${Math.round(played.elapsed)} ms: ${played.pushes} pushes, commits ${played.commits}; asked ${played.asked.join(', ') || 'nothing'}`);
    // Run 12: 118 commits in 86 frames at 1,440 h/s. Each push and each new order is structure: the widget's paced
    // structures share one slot of the page's pace, so together they commit at most four times a second.
    assert.ok(played.commits <= Math.ceil(played.elapsed / 250) + 2, `at most a commit a quarter second: ${played.commits} in ${Math.round(played.elapsed)} ms; why ${JSON.stringify(played.why)}`);
    assert.deepEqual(played.asked, [], 'a push at a new hour asks no name: the countries are named where an identity last changed');
    assert.ok((await act<(string | null)[][]>('painted()')).length > 0, 'the states are drawn and painted');
    await act('pause()');
  }, 180_000));
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';
import type { fixture } from './GameWindow.keyboard.fixture';

type Snapshot = ReturnType<typeof fixture.snapshot>;

test('a game window is browsed by keyboard and belongs to the client that asked for it', async () => {
  await withNativePage('game-window-keyboard', '/ui/src/components/GameWindow.keyboard.fixture.tsx', page => page.stage('game window keyboard', async () => {
    const act = (expression: string) => page.evaluate(`window.gameWindowFixture.${expression}`);
    const snapshot = () => act('snapshot()') as Promise<Snapshot>;
    const press = async (key: Parameters<typeof page.press>[0], shift = false) => { await page.press(key, shift); await act('settle()'); };

    // Tab reaches the window, then each row, named with every detail its tooltip shows.
    await act('mountScene()'); await act('focusFirstOutside()');
    await press('Tab');
    assert.equal((await snapshot()).active, 'Policies', 'the scrollable window itself takes focus first');
    await press('Tab');
    assert.equal((await snapshot()).active, 'Policy 0');
    await press('Tab');
    assert.equal((await snapshot()).active, 'Policy 1. Not available: its availability condition does not hold. Attached to this organisation');
    // Keys scroll the row's list: a line by the row's height, a page by the list's, the ends.
    await press('ArrowDown');
    await press('PageDown');
    await press('End');
    await press('Home');
    assert.deepEqual((await snapshot()).scrolls, [['w.list', 30], ['w.list', 100], ['w.list', 300]],
      'Home at the drawn start leaves the list where it is');
    // A row focused beyond the list's box scrolls the list just far enough, never the clip.
    await press('Tab');
    assert.equal((await snapshot()).active, 'Policy 2');
    assert.equal((await snapshot()).scrolls.length, 3, 'a row within the box scrolls nothing');
    await press('Tab');
    assert.equal((await snapshot()).active, 'Policy 3');
    await press('Tab');
    let state = await snapshot();
    assert.equal(state.active, 'Policy 4');
    assert.deepEqual(state.scrolls.slice(3), [['w.list', 20], ['w.list', 50]], 'each revealed by the list offset');
    assert.deepEqual(state.scrolled, [0, 0], 'neither the clip nor the window scrolled on its own');
    await press('Tab', true);
    assert.equal((await snapshot()).active, 'Policy 3', 'Shift+Tab goes back');

    // Another client with the same recording asks afresh rather than showing the first's window.
    await act('mountWindow("first")');
    state = await snapshot();
    assert.deepEqual(state.asked, ['first']);
    assert.equal(state.scene, 'Window of first');
    await act('mountWindow("second")');
    state = await snapshot();
    assert.deepEqual(state.asked, ['first', 'second']);
    assert.equal(state.scene, 'Window of second');

    // A row focused beyond its list's box has the window laid out again at the offset that
    // reveals it, down or across: the reply draws it within the box.
    // At a container scale of 1 and of 0.5 the revealing offset is the same 50 units; the row is
    // then drawn at 10 + 70 units times the scale, its end at the box's.
    for (const scale of [1, 0.5]) for (const horizontal of [false, true]) {
      const what = `${horizontal ? 'across' : 'down'} at scale ${scale}`;
      await act(`mountScrolling(${horizontal}, ${scale})`); await act('focusFirstOutside()');
      for (let tab = 0; tab < 6; tab++) await press('Tab');
      const revealed = await act(`repliedAt(50, ${horizontal})`) as { replied: number[]; active: string | null; start: number | null };
      assert.equal(revealed.active, 'Policy 4', `${what}: the row keeps focus`);
      assert.equal(revealed.replied[0], 0);
      assert.equal(revealed.replied[revealed.replied.length - 1], 50, `${what}: laid out at the revealing offset`);
      assert.equal(revealed.start, 10 + 70 * scale, `${what}: drawn wholly within the box`);
    }

    // Two windows colouring the same page at once both draw it, whichever finished first.
    await act('mountTwins()');
    assert.deepEqual(await act('twinQuads()'), [1, 1]);
  }, 120_000));
});

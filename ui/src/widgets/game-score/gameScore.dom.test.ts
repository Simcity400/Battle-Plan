import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

interface Played { head: number; slots: number; members: number; orders: number; commits: number; why: Record<string, number>; asked: string[] }

test('game score\'s top three as a long campaign ranks at 1,440 h/s: each change of its members commits once, nothing commits for the pushes or the scores, nothing is asked', async t => {
  await withNativePage('game-score', '/ui/src/widgets/game-score/gameScore.dom.fixture.tsx', page => page.stage('game score', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.gameScore.${expression}`);
    assert.notEqual(await act('mount()'), null, 'the widget follows its panel');
    assert.equal((await act<string[][]>('rows()')).length, 3, `the top three shown: ${await act<string>('text()')}`);
    const played = await act<Played>('play(600, 15, 15)');
    t.diagnostic(`600 frames to hour ${played.head}: ${played.members} member changes, ${played.orders} order changes, ${played.commits} commits in ${played.slots} slots ${JSON.stringify(played.why)}; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.ok(played.members > 5, 'the top\'s members move');
    assert.ok(played.commits <= played.members + played.orders, `one commit a change of the top at most (${played.commits} for ${played.members + played.orders})`);
    assert.deepEqual(played.asked, [], 'no request at the head: no name asked as the rows reorder');
    assert.equal(await act('lastShown()'), played.head, 'it reports the hour of the scores it painted');
    await act('pause()');
  }, 180_000));
});

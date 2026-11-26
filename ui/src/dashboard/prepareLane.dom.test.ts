import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

type Mounted = { asked: string[]; shownOnPause: number };
type Played = { commits: number; preparedRenders: number; asked: string[]; hostWhilePlaying: boolean };

test('at 15 hours a frame for 600 frames, the page plays the same with the prepare lane on as with it off; paused, the lane prepares every other page', async t => {
  await withNativePage('prepare-lane', '/ui/src/dashboard/prepareLane.dom.fixture.tsx', page => page.stage('prepare lane', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.prepareLane.${expression}`);
    const runs: Record<'off' | 'on', { mounted: Mounted; played: Played }> = {} as never;
    for (const lane of ['off', 'on'] as const) {
      const mounted = await act<Mounted>(`mount(${lane === 'on'})`);
      const played = await act<Played>('play(600, 15)');
      t.diagnostic(`lane ${lane}: paused asked ${mounted.asked.join(', ')}; playing: ${played.commits} commits, asked ${played.asked.join(', ') || 'nothing'}, `
        + `prepared renders ${played.preparedRenders}`);
      runs[lane] = { mounted, played };
    }
    const { off, on } = runs;
    assert.deepEqual(off.mounted.asked.filter(product => product !== 'shown'), [], 'off: nothing but the page shown is asked');
    assert.ok(['politics', 'economy'].every(product => on.mounted.asked.includes(product)), `on, paused: every other page asked: ${on.mounted.asked.join(', ')}`);
    assert.ok(off.played.asked.filter(product => product === 'shown').length >= 3, `the page asks a block as the head enters it: ${off.played.asked.join(', ')}`);
    assert.equal(on.played.preparedRenders, 0, 'playing, no prepared page renders');
    assert.deepEqual(on.played.asked, off.played.asked, 'playing, the same requests at the head, lane on or off');
    assert.equal(on.played.commits, off.played.commits, 'playing, the page commits as often, lane on or off');
    assert.equal(on.played.hostWhilePlaying, false, 'no prepared page laid out on any playing frame, the last included');
    assert.deepEqual(await act('heldFirstRender()'), { product: true, followed: true }, 'a reply the lane holds shows from the first render: no loading state, not for a frame');
    assert.deepEqual(await act('heldBetweenRenders()'), { product: true, products: true, followed: true },
      'a reply settling while its reader waits is shown by the update that hands it over, so what it starts runs');
    const followed = await act<{ acquired: string[]; seen: Record<string, { status: string; hidden: boolean }> }>('followed()');
    assert.deepEqual(followed.acquired, ['shown'], 'a prepared copy follows nothing; a shown widget follows its head');
    assert.deepEqual(followed.seen.prepared, { status: 'off', hidden: false }, 'a prepared copy reads its head as an unfollowed widget does, through the lane');
    assert.deepEqual(followed.seen['off screen'], { status: 'pending', hidden: true }, 'a shown widget off screen still reads nothing');
  }));
});

test('play pressed while the lane has a page laid out drops it in the play commit: no widget of it renders as playing', async () => {
  await withNativePage('prepare-play', '/ui/src/dashboard/prepareLane.dom.fixture.tsx', page => page.stage('prepare play', async () => {
    const played = await page.evaluate<{ laid: boolean; playingRenders: number; shown: boolean }>('window.prepareLane.playWhilePreparing()');
    assert.ok(played.laid, 'the lane had a page laid out when play was pressed');
    assert.equal(played.playingRenders, 0, `no widget of the prepared page rendered as playing: ${played.playingRenders}`);
    assert.equal(played.shown, false, 'the prepared page is gone once the play commit is painted');
  }));
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('at 15 hours a frame, the raid tally commits nothing, the raid log commits only paced structure, neither asks anything, and both report the head', async t => {
  await withNativePage('raid-playback', '/ui/src/widgets/raid-log/raidPlayback.dom.fixture.tsx', page => page.stage('raid playback', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.raidPlayback.${expression}`);
    await act('mount()');
    assert.deepEqual(await act('shown()'), { tally: 30_000, log: 30_000 }, 'paused, each shows the head\'s hour');
    assert.match(await act<string>('text()'), /prepared/, 'a preparing raid shows its hours');

    const played = await act<{ head: number; asked: string[]; rendered: Record<string, number>; elapsed: number }>('play(600, 15)');
    t.diagnostic(`600 frames to hour ${played.head} in ${Math.round(played.elapsed)} ms: renders ${JSON.stringify(played.rendered)}; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.equal(played.rendered.tally ?? 0, 0, 'the race: the same three rows, values painted, no commit');
    // The widget renders at most four times a second, when a raid's phase starts or stops being read, and the shared
    // list commits nothing after it.
    assert.ok((played.rendered.log ?? 0) <= Math.ceil(played.elapsed / 250) + 2, `the log: structure paced to four a second, ${played.rendered.log} commits`);
    assert.deepEqual(played.asked.filter(product => product === 'raid-log' || product === 'place-names'), [], 'no raid or name read while playing');
    assert.deepEqual(await act('shown()'), { tally: played.head, log: played.head }, 'each reports the hour it paints');
    await act('pause()');
  }, 180_000));
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('Politics at 15 hours a frame: National focus, Stability and Political power spending ask nothing at the head and commit only where what they show changes', async t => {
  await withNativePage('politics-playback', '/ui/src/widgets/national-focus/politicsPlayback.dom.fixture.tsx', page => page.stage('politics playback', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.politics.${expression}`);
    await act('mount()');
    assert.deepEqual(await act('errors()'), []);
    t.diagnostic(`mounted, asked: ${(await act<string[]>('asked()')).join('; ')}`);

    const titles = await act<{ name: string; static: boolean }[]>('titleReads()');
    assert.deepEqual(titles.filter(title => title.static).map(title => title.name), [], 'a focus with a static title is never read dated');

    type Played = { head: number; rendered: Record<string, number>; causes: Record<string, Record<string, number>>; atHead: string[]; ahead: string[] };
    const played = await act<Played>('play(600, 15)');
    t.diagnostic(`600 frames to hour ${played.head}: renders ${JSON.stringify(played.rendered)}; why ${JSON.stringify(played.causes)}`);
    t.diagnostic(`rows: ${JSON.stringify(await act('rows()'))}`);
    t.diagnostic(`at the head: ${played.atHead.join('; ') || 'nothing'}`);
    t.diagnostic(`ahead: ${played.ahead.join('; ') || 'nothing'}`);
    assert.deepEqual(await act('errors()'), []);
    assert.deepEqual(played.atHead, [], 'no request for values at the head while playing');
    // 9,000 hours played: eleven focuses complete, each a change of what National focus lists (one commit each, at the
    // page's quarter-second slot), and the art of what comes next is asked a stretch of playback at a time (its reads
    // land with one commit a stretch). The list's own measuring and scrolling (VirtualList) is the shared list's.
    const completions = Math.ceil(9_000 / 840), stretches = Math.ceil(9_000 / 5_760) + 1;
    const own = Object.entries(played.causes.focus ?? {}).filter(([cause]) => cause.startsWith('NationalFocusWidget')).reduce((sum, [, count]) => sum + count, 0);
    assert.ok(own <= completions + stretches, `National focus began ${own} commits for ${completions} completions and ${stretches} stretches`);
    assert.equal(played.rendered.stability ?? 0, 0, `Stability: ${played.rendered.stability} commits while its values are followed`);
    const focus = await act<{ current: string | null; completed: number; newest: string | null }>('focus()');
    assert.ok(focus.completed > 0, 'the list shows completions');
    t.diagnostic(`spending: ${JSON.stringify(await act('spending()'))}`);
    // Political power spending draws a block of two seconds of payments at once (at most until a month starts): one
    // commit a block, the names its rows show asked in it, never a second commit after it.
    const blocks = Math.ceil(9_000 / 2_880) + 1;
    assert.ok((played.rendered.spending ?? 0) <= blocks, `Political power spending: ${played.rendered.spending} commits for at most ${blocks} blocks`);
    assert.deepEqual(await act('shown()'), { focus: played.head, stability: played.head, spending: played.head }, 'each reports the hour it shows');

    // A setting changed while playing renders Stability at a new head: its names, colours and flags stay at the hour
    // an identity last changed, so nothing is asked.
    const restyled = await act<Played>('play(120, 15, 60)');
    t.diagnostic(`after a setting: renders ${JSON.stringify(restyled.rendered)}; at the head ${restyled.atHead.join("; ") || "nothing"}; ahead ${restyled.ahead.join("; ") || "nothing"}`);
    assert.deepEqual(restyled.atHead, [], 'a render while playing asks nothing at the head');
    // Its root's ref is the same function each render: a new one set the follow root to null and back, a second commit.
    assert.equal(restyled.rendered.stability, 1, 'one setting changed, one commit');
    await act('pause()');
  }, 240_000));
});

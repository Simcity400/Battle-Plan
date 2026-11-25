import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('place names landing after the block is drawn: the rows in view are asked first, in a read of their own, and show their names within a frame of it landing, paused and playing', async t => {
  await withNativePage('place-names', '/ui/src/widgets/battles/placeNames.dom.fixture.tsx', page => page.stage('place names', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.placeNames.${expression}`);
    type Shown = { province: number; place: string }[];
    type Ask = { land: boolean; provinces: number[]; landed: boolean };
    const shown = () => act<Shown>('landShown()');
    await act('mount()');
    const paused = await shown();
    assert.ok(paused.length >= 5, `land battles draws its rows: ${JSON.stringify(paused)}`);
    assert.ok(paused.every(row => row.place === 'Loading…'), `no name has landed: ${JSON.stringify(paused)}`);
    const asked = await act<Ask[]>('asks()');
    assert.equal(asked.length, 1, 'the rest wait for the visible land battles to be named');
    const ask = asked[0]!;
    assert.ok(ask.land && paused.every(row => ask.provinces.includes(row.province)) && ask.provinces.length <= 24,
      `the visible battle places are asked first: ${ask.provinces.join(', ')}`);
    type Landing = { frames: number; commits: Record<string, number> };
    const landing = await act<Landing>('landAndCount([0])');
    t.diagnostic(`paused: named ${landing.frames} frames after landing, commits ${JSON.stringify(landing.commits)}`);
    assert.ok(landing.frames <= 1, `paused, every row in view is named in the frame its read lands: ${landing.frames} frames ${JSON.stringify(await shown())}`);
    assert.ok((landing.commits.land ?? 0) <= 1, `a landing costs a commit a widget: ${JSON.stringify(landing.commits)}`);
    await act('settle(4)');
    const rest = (await act<Ask[]>('asks()')).slice(1);
    assert.ok(rest.length >= 1 && rest.every(ask => !ask.landed), `the rest of the places are asked once the rows in view are named: ${rest.length} reads`);
    await act('play(48, 15)');
    const playing = await shown();
    assert.ok(playing.some(row => row.province > 300 && row.place === 'Loading…'), `battles begun while playing wait for their read: ${JSON.stringify(playing)}`);
    const reads = await act<Ask[]>('asks()');
    const out = reads.flatMap((ask, index) => ask.landed ? [] : [index]);
    const played = await act<Landing>(`landAndCount([${out.join(', ')}], 15)`);
    t.diagnostic(`playing: named ${played.frames} frames after landing, commits ${JSON.stringify(played.commits)}`);
    assert.ok(played.frames <= 1, `playing, every row in view is named in the frame its read lands, painted: ${played.frames} frames ${JSON.stringify(await shown())}`);
    assert.ok((played.commits.land ?? 0) <= 1, `playing, a landing is painted, not committed: ${JSON.stringify(played.commits)}`);
    await act('play(30, 15)');
    const after = await shown();
    assert.ok(after.filter(row => row.province <= 300).every(row => row.place === `Province ${row.province}`),
      `once React has taken the names, the rows still show them: ${JSON.stringify(after)}`);
    await act('pause()');
  }, 120_000));
});

test('following at 1,440 h/s over windows of 720 hours, every window, place and terrain read is asked ahead of the head, each once', async t => {
  await withNativePage('battle-windows', '/ui/src/widgets/battles/battleWindows.dom.fixture.tsx', page => page.stage('battle windows', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.battleWindows.${expression}`);
    await act('mount()');
    assert.ok(await act<number>('rows()') > 0, 'the log draws its battles');
    await act('play(600, 15)');
    type Asked = { product: string; window: number | null; from: number | null; head: number };
    const played = await act<Asked[]>('played()');
    const windows = played.filter(asked => asked.product === 'land-battles');
    t.diagnostic(`to hour ${await act<number>('head()')}: ${windows.length} windows asked (${windows.map(asked => `${asked.window}@${asked.head}`).join(', ')}); ${played.length} requests`);
    assert.ok(windows.length >= 10, `the windows the head crossed were asked while playing: ${windows.length}`);
    const atHead = played.filter(asked => asked.product !== 'follow' && asked.product !== 'follow-clock' && asked.product !== 'unfollow' && (asked.from === null || asked.from <= asked.head));
    assert.deepEqual(atHead, [], 'no request asks for values at the head while playing');
    assert.equal(new Set(windows.map(asked => asked.window)).size, windows.length, 'no window asked twice');
    await act('pause()');
  }, 120_000));
});

test('Land battles at a world war\'s density, following at 1,440 h/s: rows are bound by the frame loop into the same boxes, never mounted, each state once', async t => {
  await withNativePage('land-playback', '/ui/src/widgets/land-battles/landPlayback.dom.fixture.tsx', page => page.stage('land playback', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.landPlayback.${expression}`);
    await act('mount()');
    assert.ok(await act<number>('rows()') > 0, 'the log draws its battles');
    const played = await act<{ commits: { ms: number; kept: number; redrawn: number }[]; why: Record<string, number>; errors: string[]; pooled: boolean; sameBoxes: boolean;
      bindings: number; boundStates: number }>('play(300, 15)');
    const sum = (pick: (commit: { ms: number; kept: number; redrawn: number }) => number) => played.commits.reduce((total, commit) => total + pick(commit), 0);
    t.diagnostic(`${played.commits.length} commits, ${(sum(commit => commit.ms) / Math.max(1, played.commits.length)).toFixed(1)} ms each in React; rows kept ${sum(commit => commit.kept)}, elements drawn again inside them ${sum(commit => commit.redrawn)}; ${played.bindings} bindings of ${played.boundStates} states; why ${JSON.stringify(played.why)}`);
    assert.deepEqual(played.errors, [], 'every request was served');
    assert.ok(played.commits.length >= 5 && sum(commit => commit.kept) > 0, `blocks drew rows the block before had drawn: ${played.commits.length} commits`);
    assert.equal(sum(commit => commit.redrawn), 0, 'nothing inside a row a block keeps is attached again (a battle\'s new state aside)');
    assert.ok(played.pooled && played.sameBoxes, 'following, the rows are the list\'s pooled boxes, the same elements from the first frame to the last');
    assert.deepEqual(Object.keys(played.why).filter(cause => cause.startsWith('BattleCells')), [], 'no battle row is rendered by React while following');
    assert.ok(played.bindings > 0 && played.bindings <= 2 * played.boundStates,
      `each state of a row is bound about once (again only when its names land): ${played.bindings} bindings of ${played.boundStates} states`);
  }, 180_000));
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('the scoreboard at 15 hours a frame: values painted with no commit and no request, the hour shown reported', async t => {
  await withNativePage('scoreboard', '/ui/src/widgets/scoreboard/scoreboard.dom.fixture.tsx', page => page.stage('scoreboard', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.scoreboard.${expression}`);
    await act('mount()');
    assert.deepEqual(await act('follows()'), ['dealt', 'free7', 'partial', 'rank'], 'the top followed by rank, every column and the partial mark followed for it');
    assert.equal(await act('shown()'), 30_000, 'paused, it shows the head\'s hour');
    const text = await act<string>('text()');
    assert.match(text, /3,0\d\d/, 'GER\'s factories painted');
    assert.match(text, /† 20,\d\d\d/, 'SOV\'s partial casualties marked');

    await act('wait(3000)');
    const played = await act<{ head: number; asked: string[]; rendered: number }>('play(600, 15)');
    t.diagnostic(`600 frames to hour ${played.head}: ${played.rendered} commits; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.equal(played.rendered, 0, 'no commit as the head moved: the rows and their order held');
    assert.deepEqual(played.asked, [], 'no request for values at the head');
    assert.equal(await act('shown()'), played.head, 'it reports the hour of the last values painted');
    const factories = 3_000 + played.head % 97;
    assert.ok((await act<string>('text()')).includes(factories.toLocaleString('en-US')), `GER's factories at the head (${factories}) painted`);
    await act('pause()');
  }, 180_000));
});

test('the scoreboard as a long campaign ranks: the top three\'s members and order moving, each change commits once, at most once a quarter second', async t => {
  await withNativePage('scoreboard-churn', '/ui/src/widgets/scoreboard/scoreboard.dom.fixture.tsx', page => page.stage('scoreboard churn', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.scoreboard.${expression}`);
    await act('mount(true)');
    await act('wait(3000)');
    const started = Date.now();
    const played = await act<{ head: number; asked: string[]; rendered: number; why: Record<string, number>; members: number; orders: number }>('play(600, 15)');
    const slots = Math.ceil((Date.now() - started) / 250) + 1;
    t.diagnostic(`600 frames: ${played.members} member changes, ${played.orders} order changes, ${played.rendered} commits in ${slots} slots ${JSON.stringify(played.why)}; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.ok(played.rendered <= Math.min(slots, played.members + played.orders), `one commit a structural change at most, and one a slot (${played.rendered})`);
    assert.deepEqual(played.asked, [], 'no request for values at the head');
    await act('pause()');
  }, 180_000));
});

test('the scoreboard following a top of 1,500 countries follows every column for all of them', async () => {
  await withNativePage('scoreboard-1500', '/ui/src/widgets/scoreboard/scoreboard.dom.fixture.tsx', page => page.stage('scoreboard 1500', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.scoreboard.${expression}`);
    await act('mount(false, 1500)');
    assert.deepEqual(await act('follows()'), ['dealt', 'free7', 'partial', 'rank'], 'the top followed by rank, every column followed for it');
    assert.deepEqual(await act('followedTags()'), { dealt: 1_500, free7: 1_500, partial: 1_500 }, 'each column reads every country of the top');
    assert.match(await act<string>('text()'), /1,500,0\d\d/, 'the leading country\'s factories painted at the head');
  }, 180_000));
});

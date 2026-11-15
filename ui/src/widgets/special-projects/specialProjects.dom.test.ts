import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('at 15 hours a frame for 600 frames, special projects ask nothing, commit only when a project starts, paint the head progress and the pushed details, and report the hour they show', async t => {
  await withNativePage('special-projects', '/ui/src/widgets/special-projects/specialProjects.dom.fixture.tsx', page => page.stage('special projects', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.specialProjects.${expression}`);
    await act('mount()');
    assert.deepEqual(await act('shown()'), { current: 30_000, history: 30_000 }, 'paused, each shows the head\'s hour');
    assert.deepEqual(await act<{ id: string }[]>('cards()').then(cards => cards.map(card => card.id)), ['fast', 'slow'], 'the late project has not started');

    const played = await act<{ head: number; asked: string[]; rendered: Record<string, number> }>('play(600, 15)');
    t.diagnostic(`600 frames to hour ${played.head}: renders ${JSON.stringify(played.rendered)}; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.deepEqual(played.asked, [], 'no request while playing: the head is read from the reply already held');
    // The late project starting at 34,000 is the one structural change on either view; daily progress steps are painted.
    assert.ok((played.rendered.current ?? 0) <= 2, `Current: ${played.rendered.current} commits for one project starting`);
    assert.ok((played.rendered.history ?? 0) <= 2, `History: ${played.rendered.history} commits for one project starting`);
    const cards = await act<{ id: string; progress: string | null }[]>('cards()');
    assert.deepEqual(cards.map(card => card.id), ['fast', 'slow', 'late']);
    for (const card of cards) assert.equal(card.progress, await act<string | null>(`progressAt(${JSON.stringify(card.id)}, ${played.head})`), `${card.id} painted at the head`);
    assert.deepEqual(await act('shown()'), { current: played.head, history: played.head }, 'each reports the hour it painted');
    // The details' panel was pushed every day's read while playing: its values are painted, never a render.
    const read = played.head - (played.head - 4) % 24;
    const painted = await act<Record<string, string>>('painted()');
    assert.equal(painted['b:0'], `: ${Math.floor(read / 24) * 100 / 10000} breakthrough progress · read ${painted['s:7:injury']?.split(' · read ')[1]}`, 'the last pushed read is painted');
    assert.match(painted['s:7:injury'] ?? '', new RegExp(`^${Math.max(0, 400 - Math.floor(read / 24) % 400)} injury recovery days · read `));
    await act('pause()');
  }, 180_000));
});

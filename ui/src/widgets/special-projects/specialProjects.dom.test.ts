import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

type Act = <T>(expression: string) => Promise<T>;
const acting = (page: { evaluate: <T>(expression: string) => Promise<T> }): Act => expression => page.evaluate(`window.specialProjects.${expression}`);

test('at 15 hours a frame for 600 frames, special projects ask nothing, commit only when what they draw changes, paint the head progress and the pushed details, and report the hour they show', async t => {
  await withNativePage('special-projects', '/ui/src/widgets/special-projects/specialProjects.dom.fixture.tsx', page => page.stage('special projects', async () => {
    const act = acting(page);
    await act('mount()');
    assert.deepEqual(await act('shown()'), { current: 30_000, history: 30_000 }, 'paused, each shows the head\'s hour');
    assert.deepEqual(await act<{ id: string }[]>('cards()').then(cards => cards.map(card => card.id)), ['fast', 'slow'], 'the late project has not started');

    const played = await act<{ head: number; asked: string[]; rendered: Record<string, number> }>('play(600, 15)');
    t.diagnostic(`600 frames to hour ${played.head}: renders ${JSON.stringify(played.rendered)}; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.deepEqual(played.asked, [], 'no request while playing: each reads the panel it follows');
    assert.ok((played.rendered.current ?? 0) <= 3, `Current: ${played.rendered.current} commits for one project starting and one completion listed`);
    const starts = await act<number>(`statusStarts(30000, ${played.head})`);
    assert.ok((played.rendered.history ?? 0) <= 2 * starts + 2, `History: ${played.rendered.history} commits for ${starts} new statuses, at most the install and the picture for each`);
    const cards = await act<{ id: string; progress: string | null }[]>('cards()');
    assert.deepEqual(cards.map(card => card.id), ['fast', 'slow', 'late']);
    for (const card of cards) assert.equal(card.progress, await act<string | null>(`progressAt(${JSON.stringify(card.id)}, ${played.head})`), `${card.id} painted at the head`);
    await act('pause()');
    await act('settled()');
    assert.deepEqual(await act('shown()'), { current: played.head, history: played.head }, 'paused at the head, each reports the hour it painted');
    const read = played.head - (played.head - 4) % 24;
    const painted = await act<Record<string, string>>('painted()');
    assert.equal(painted['b:0'], `: ${Math.floor(read / 24) * 100 / 10000} breakthrough progress · read ${painted['s:7:injury']?.split(' · read ')[1]}`, 'the last pushed read is painted');
    assert.match(painted['s:7:injury'] ?? '', new RegExp(`^${Math.max(0, 400 - Math.floor(read / 24) % 400)} injury recovery days · read `));
  }, 180_000));
});

test('U-S2, R3-S3, R3-S6: History tooltips, menus and errors come from its followed panel, never a whole-branch reply, and the Current menu reads the installed reply', async () => {
  await withNativePage('special-projects', '/ui/src/widgets/special-projects/specialProjects.dom.fixture.tsx', page => page.stage('special projects wiring', async () => {
    const act = acting(page);
    await act('mount()');
    assert.ok(!(await act<string[]>('asked()')).includes('special-projects'), 'R3-S3: History asks no whole-branch reply while it follows');
    const played = await act<{ head: number }>('play(240, 15)');
    await act('pause()');
    await act('settled()');
    const currentMenu = await act<string[] | null>('menuFor("fast")');
    assert.ok(currentMenu?.some(item => item.startsWith('Jump to completion')), `R3-S6: a completion listed after the card was drawn is in its menu: ${currentMenu?.join(' | ')}`);
    assert.equal(await act('historyKey("ArrowDown")'), true, 'History draws its timeline');
    await act('settled()');
    const tip = await act<{ title: string | null; rows: [string, string][] } | null>('tooltip()');
    assert.equal(tip?.title, 'Rocket engines', `U-S2: the History tooltip reads the followed reply, never the whole-branch one: ${JSON.stringify(tip)}`);
    assert.equal(tip?.rows.find(([label]) => label === 'Progress')?.[1], await act<string | null>(`progressAt("fast", ${played.head})`), 'and its progress is the installed one at the head');
    await act('historyKey("ContextMenu")');
    const historyMenu = await act<string[]>('menu()');
    assert.ok(historyMenu.some(item => item.startsWith('Jump to first recorded')), `the History menu opens on the followed project: ${historyMenu.join(' | ')}`);

    await act('mount("Projects refused in this fixture")');
    await act('settled()');
    const refusal = await act<string>('text("history")');
    assert.match(refusal, /Projects refused in this fixture/, `R3-S3, F-S5: History shows its followed panel's refusal by name: ${refusal}`);
  }, 180_000));
});

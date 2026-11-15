import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

interface Played { head: number; pushes: number; commits: number; why: Record<string, number>; asked: string[]; tiles: number }

test('world tension as the recording pushes it at 1,440 h/s: countries below its five layers trading places at every push commit nothing; the values are painted, nothing is asked, the hour shown is reported', async t => {
  await withNativePage('world-tension', '/ui/src/widgets/world-tension/worldTension.dom.fixture.tsx', page => page.stage('world tension', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.worldTension.${expression}`);
    assert.notEqual(await act('mount()'), null, 'the widget follows its panel');
    const before = await act<string[]>('list()');
    assert.equal(before.length, 6, 'five layers and Other');
    assert.match(before[0], /\d%$/, 'the greatest contribution painted');
    assert.match(before[5], /^Other \(25\)/, 'Other counts the 25 countries outside the layers');
    const played = await act<Played>('play(600, 15, 15)');
    t.diagnostic(`600 frames, ${played.pushes} pushes to hour ${played.head}: ${played.commits} commits ${JSON.stringify(played.why)}; asked ${played.asked.join(', ') || 'nothing'}`);
    assert.equal(played.commits, 0, 'no commit: the layers, their order and Other\'s count held while the countries below them moved');
    assert.deepEqual(played.asked, [], 'no request for values at the head');
    assert.match(await act<string>('total()'), /^40(\.\d)?%$/, 'the tension at the head painted');
    assert.equal(await act('lastShown()'), played.head, 'it reports the hour of the cells it painted');
    await act('pause()');
  }, 180_000));
});

test('world tension whose layers trade places at every hour: their order commits once a quarter second at most', async t => {
  await withNativePage('world-tension-moving', '/ui/src/widgets/world-tension/worldTension.dom.fixture.tsx', page => page.stage('world tension moving', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.worldTension.${expression}`);
    assert.notEqual(await act('mount(true)'), null, 'the widget follows its panel');
    const played = await act<Played>('play(600, 15, 15)');
    t.diagnostic(`600 frames, ${played.pushes} pushes: ${played.commits} commits ${JSON.stringify(played.why)}; ${played.tiles} tiles asked`);
    assert.ok(played.commits > 0, 'the layers\' new order is drawn');
    assert.ok(played.commits <= played.pushes, `at most one commit a push (${played.commits} for ${played.pushes})`);
    assert.ok(played.tiles < played.pushes / 4, `the same countries in another order ask no line again (${played.tiles} tiles asked)`);
    await act('pause()');
  }, 180_000));
});

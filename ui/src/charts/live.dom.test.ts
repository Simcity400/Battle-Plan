import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

type Act = <T>(expression: string) => Promise<T>;
type Commits = Record<string, number>;

const since = async (act: Act, chart: string, before: Commits) => ((await act<Commits>('commits()'))[chart] ?? 0) - (before[chart] ?? 0);
const scores = (times: number, skip?: string) => JSON.stringify(Object.fromEntries(Array.from({ length: 200 }, (_, i) => [`r${i}`, i * 10 * times])
  .filter(([key]) => key !== skip)));

test('the shared charts paint live values without a commit, draw a value the source lacks as missing, and are unchanged without a source', async t => {
  await withNativePage('live-charts', '/ui/src/charts/live.dom.fixture.tsx', async page => {
    const act: Act = expression => page.evaluate(`window.liveCharts.${expression}`);

    await t.test('table: painted cells and bars move, a missing value is a dash with its reason, a live sort re-orders with a commit', () => page.stage('table', async () => {
      await act('mountTable(true, "name")');
      await act(`pushTable(${scores(1)})`);
      assert.deepEqual((await act<{ text: string }[]>('tableRows(3)')).map(row => row.text), ['0 pts', '10 pts', '20 pts']);
      assert.ok(await act<number>('tableMounted()') < 200, 'rows out of view are not mounted, so not painted');
      const before = await act<Commits>('commits()');
      for (let times = 2; times <= 11; times++) await act(`pushTable(${scores(times)})`);
      const rows = await act<{ key: string; text: string; bar: string }[]>('tableRows(3)');
      assert.deepEqual(rows.map(row => row.text), ['0 pts', '110 pts', '220 pts']);
      assert.ok(Math.abs(parseFloat(rows[1]!.bar) - 110 / (1990 * 11) * 100) < 1e-3, `the bar is a share of the greatest live value: ${rows[1]!.bar}`);
      assert.equal(await since(act, 'table', before), 0, 'ten pushes of values committed nothing');
      await act(`pushTable(${scores(12, 'r1')})`);
      const missing = (await act<{ text: string; title: string; bar: string }[]>('tableRows(3)'))[1]!;
      assert.deepEqual([missing.text, missing.title, missing.bar], ['—', 'no score for r1', '0%'], 'never zero, never the last value');
      assert.equal(await since(act, 'table', before), 0);

      await act('mountTable(false, "name")');
      const plain = await act<Commits>('commits()');
      await act(`pushTable(${scores(13)})`);
      assert.deepEqual((await act<{ text: string }[]>('tableRows(3)')).map(row => row.text), ['0', '1', '2'], 'without a source the column renders its value');
      assert.equal(await since(act, 'table', plain), 0);

      await act(`pushTable(${scores(1)})`);
      await act('mountTable(true, "score")');
      assert.deepEqual((await act<{ key: string }[]>('tableRows(2)')).map(row => row.key), ['r199', 'r198']);
      const sorted = await act<Commits>('commits()');
      const reversed = JSON.stringify(Object.fromEntries(Array.from({ length: 200 }, (_, i) => [`r${i}`, (200 - i) * 10])));
      await act(`pushTable(${reversed})`);
      assert.deepEqual((await act<{ key: string }[]>('tableRows(2)')).map(row => row.key), ['r0', 'r1'], 'the order follows the live values');
      assert.ok(await since(act, 'table', sorted) >= 1, 'a new order is a commit');
    }));

    await t.test('timeline: the playhead follows the live head without a commit, and the prop without one', () => page.stage('timeline', async () => {
      await act('mountTimeline(true)');
      const start = await act<number>('playheadX()');
      assert.ok(Number.isFinite(start));
      const before = await act<Commits>('commits()');
      await act('pushHead(500)');
      const middle = await act<number>('playheadX()');
      await act('pushHead(900)');
      const late = await act<number>('playheadX()');
      assert.ok(start < middle && middle < late, `the playhead moved right: ${start}, ${middle}, ${late}`);
      assert.equal(await since(act, 'timeline', before), 0);

      await act('mountTimeline(false)');
      const fixed = await act<number>('playheadX()');
      const plain = await act<Commits>('commits()');
      await act('pushHead(200)');
      assert.equal(await act<number>('playheadX()'), fixed, 'without a live head the playhead stays at the prop');
      assert.equal(fixed, start);
      assert.equal(await since(act, 'timeline', plain), 0);
    }));

    await t.test('sunburst: centre and legend amounts move without a commit, a missing whole shows as not recorded, a new part is one commit', () => page.stage('sunburst', async () => {
      await act('mountSunburst(true)');
      await act('pushSunburst({ items: window.liveCharts.items(10, 20) })');
      assert.ok((await act<{ canvas: string[] }>('sunburst()')).canvas.includes('A30'));
      const before = await act<Commits>('commits()');
      await act('pushSunburst({ items: window.liveCharts.items(15, 45) })');
      const moved = await act<{ canvas: string[]; legend: string[][] }>('sunburst()');
      assert.ok(moved.canvas.includes('A60'), `the centre's amount: ${moved.canvas.join(' | ')}`);
      assert.deepEqual(moved.legend.map(row => row.slice(0, 2)), [['Beta', 'A45'], ['Alpha', 'A15']]);
      assert.equal(await since(act, 'sunburst', before), 0, 'values moved and nothing committed');
      await act('pushSunburst({ items: [], unavailableReason: "Not recorded at this hour" })');
      assert.ok((await act<{ canvas: string[] }>('sunburst()')).canvas.includes('Not recorded'), 'a whole the source cannot tell is not recorded, never zero');

      await act('pushSunburst({ items: window.liveCharts.items(10, 20) })');
      await act('mountSunburst(true)');
      const fresh = await act<Commits>('commits()');
      await act('pushSunburst({ items: window.liveCharts.items(15, 45, 5) })');
      assert.deepEqual((await act<{ legend: string[][] }>('sunburst()')).legend.map(row => row[0]), ['Beta', 'Alpha', 'Gamma'], 'a part that appears joins the legend');
      assert.ok(await since(act, 'sunburst', fresh) >= 1);

      await act('mountSunburst(false)');
      const plain = await act<Commits>('commits()');
      await act('pushSunburst({ items: window.liveCharts.items(100, 100) })');
      const unchanged = await act<{ canvas: string[]; legend: string[][] }>('sunburst()');
      assert.ok(unchanged.canvas.includes('A30'));
      assert.deepEqual(unchanged.legend.map(row => row.slice(0, 2)), [['Beta', 'A20'], ['Alpha', 'A10']]);
      assert.equal(await since(act, 'sunburst', plain), 0);
    }));

    await t.test('drill parts: amounts move without a commit, a missing part is not known and its row re-orders once', () => page.stage('drill', async () => {
      await act('mountDrill(true)');
      const before = await act<Commits>('commits()');
      await act('pushDrill(31, 11)');
      assert.deepEqual(await act('drill()'), [['Alpha', '31 units'], ['Beta', '11 units']]);
      assert.equal(await since(act, 'drill', before), 0);
      await act('pushDrill(null, 12)');
      assert.deepEqual(await act('drill()'), [['Beta', '12 units'], ['Alpha', 'Not known']], 'unknown goes last, shown as not known');
      assert.ok(await since(act, 'drill', before) >= 1, 'the new order is a commit');

      await act('mountDrill(false)');
      const plain = await act<Commits>('commits()');
      await act('pushDrill(1, 2)');
      assert.deepEqual(await act('drill()'), [['Alpha', '30 units'], ['Beta', '10 units']]);
      assert.equal(await since(act, 'drill', plain), 0);
    }));

    await t.test('chord: ribbons redrawn from the live frame without a commit, a missing frame not drawn as the last one', () => page.stage('chord', async () => {
      await act('mountChord(true)');
      const first = await act<string>('picture("chord")');
      const before = await act<Commits>('commits()');
      await act('pushChord(30)');
      const wider = await act<string>('picture("chord")');
      assert.notEqual(wider, first, 'a wider ribbon is drawn');
      await act('pushChord(null)');
      assert.notEqual(await act<string>('picture("chord")'), wider, 'a frame the source does not hold is not the last one drawn');
      assert.equal(await since(act, 'chord', before), 0);

      await act('mountChord(false)');
      const fixed = await act<string>('picture("chord")');
      const plain = await act<Commits>('commits()');
      await act('pushChord(50, 5)');
      assert.equal(await act<string>('picture("chord")'), fixed);
      assert.equal(await since(act, 'chord', plain), 0);
    }));

    await t.test('flow: ribbons redrawn from live values without a commit, missing values say why', () => page.stage('flow', async () => {
      await act('mountFlow(true)');
      const first = await act<string>('picture("flow")');
      const before = await act<Commits>('commits()');
      await act('pushFlow(30)');
      assert.notEqual(await act<string>('picture("flow")'), first);
      await act('pushFlow(null)');
      assert.ok((await act<string[]>('flowTexts()')).includes('Not recorded at this hour'));
      assert.equal(await since(act, 'flow', before), 0);

      await act('mountFlow(false)');
      const fixed = await act<string>('picture("flow")');
      const plain = await act<Commits>('commits()');
      await act('pushFlow(50)');
      assert.equal(await act<string>('picture("flow")'), fixed);
      assert.equal(await since(act, 'flow', plain), 0);
    }));
  });
});

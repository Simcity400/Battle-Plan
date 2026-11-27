import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

const witnesses = { 'market-ledger': 'ev:ledger-witness', 'peace-deals': 'group:peace-witness', 'division-log': 'group:division-witness' };
type Kind = keyof typeof witnesses;
type Result = { asked: string[]; shown: { kind: Kind; rows: string[]; columns: string[]; text: string; canvas: boolean }[] };

test('each ordered pair of shared log renderers asks for and draws its own product, on the page and during preparation', async t => {
  await withNativePage('log-isolation', '/ui/src/widgets/lazyWidget.dom.fixture.tsx', page => page.stage('log isolation', async () => {
    const wrong = new Set<Kind>();
    const failures: string[] = [];
    const pairs = (Object.keys(witnesses) as Kind[]).flatMap(first =>
      (Object.keys(witnesses) as Kind[]).filter(second => second !== first).map(second => [first, second, first]));
    const sequences: Kind[][] = [['market-ledger', 'peace-deals', 'division-log'], ...pairs];
    for (const prepared of [false, true]) for (const sequence of sequences) {
      await page.reload();
      const result = await page.evaluate<Result>(`window.logIsolation(${JSON.stringify(sequence)}, ${prepared})`);
      for (const shown of result.shown) {
        if (!shown.canvas || shown.rows.length !== 1 || shown.rows[0] !== witnesses[shown.kind]
          || result.asked.filter(product => product === `${shown.kind}-tail`).length !== 1) {
          wrong.add(shown.kind);
          failures.push(`${sequence.join(' then ')}, prepared ${prepared}: ${JSON.stringify(result)}`);
        }
      }
    }
    t.diagnostic(`Log widgets showing another log's rows: ${wrong.size} of ${Object.keys(witnesses).length}`);
    assert.deepEqual(failures, []);
  }, 180_000));
});

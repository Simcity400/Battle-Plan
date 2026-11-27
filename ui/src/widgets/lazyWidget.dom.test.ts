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
    for (const prepared of [false, true]) for (const first of Object.keys(witnesses) as Kind[]) for (const second of Object.keys(witnesses) as Kind[]) {
      if (first === second) continue;
      await page.reload();
      const result = await page.evaluate<Result>(`window.logIsolation(${JSON.stringify([first, second])}, ${prepared})`);
      for (const shown of result.shown) {
        if (!shown.canvas || !shown.rows.includes(witnesses[shown.kind]) || !result.asked.includes(`${shown.kind}-tail`)) {
          wrong.add(shown.kind);
          failures.push(`${first} then ${second}, prepared ${prepared}: ${JSON.stringify(result)}`);
        }
      }
    }
    t.diagnostic(`Log widgets showing another log's rows: ${wrong.size} of ${Object.keys(witnesses).length}`);
    assert.deepEqual(failures, []);
  }, 180_000));
});

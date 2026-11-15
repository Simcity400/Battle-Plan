import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';
import { glance, RECORDED_FIXTURE, recordedPage } from './recordedDriver.ts';

type WidgetState = { kind: string; state: string | null; text: string | null };

// The recorded replies reproduce the app's pages: each laid out at the app's sizes, asking exactly what the app asked
// there, in the order the app's glance visited them on one client (what a page found held from the pages before it is
// held here too). What the engine's tests and its old-against-new equality check stand on.
test('every page at mid-recording on the 13-year recording\'s replies is laid out as the app laid it out and asks only what the app asked', async t => {
  await withNativePage('recorded-pages', RECORDED_FIXTURE, native => native.stage('pages', async () => {
    const act = await recordedPage(native);
    await act('start()');
    const pages = await act<string[]>('pages()');
    assert.ok(pages.length > 0, 'the recording holds a glance at the pages');
    for (const page of pages) await t.test(page, async sub => {
      const glanced = await glance<{ hour: number; states: WidgetState[]; asked: number; misses: string[] }>(act, page);
      sub.diagnostic(`${page} at ${glanced.hour}: ${glanced.asked} requests, ${glanced.misses.length} not recorded; `
        + glanced.states.map(item => `${item.kind}: ${item.state ?? 'drawn'}${item.text ? ` (${item.text.slice(0, 80)})` : ''}`).join('; '));
      assert.deepEqual(await act(`sizes(${JSON.stringify(page)})`), [], 'every widget is laid out at the size the app gave it');
      assert.deepEqual(glanced.misses, [], 'every request of the page is one the app made');
    });
  }, 900_000));
});

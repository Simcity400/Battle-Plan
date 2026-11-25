import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Chosen { readonly view: string; readonly measure: unknown; readonly war: unknown }
interface Result { loading: { pending: string | null; rows: number }; loaded: string | null; rebuilt: number; stored: Record<string, unknown>; before: Chosen; after: Chosen; countries: unknown; title: string }

test('R4-S1: choosing a war in War contribution’s real subject menu, By measure with a saved measure, changes the war through the one option setter and keeps the view, the measure and the rest of the widget', async () => {
  await withNativePage('subject-menu', '/ui/src/dashboard/subjectMenu.dom.fixture.tsx', page => page.stage('subject menu', async () => {
    const result = await page.evaluate<Result>('window.subjectMenu.chooseWar()');
    assert.deepEqual(result.loading, { pending: 'Loading wars…', rows: 1 }, 'before the wars arrive the menu says so beside its one standing choice, never a finished list');
    assert.equal(result.loaded, null, 'the war list replaces the loading line');
    assert.deepEqual(result.before, { view: 'measure', measure: 'casualties', war: '' });
    assert.equal(result.rebuilt, 0, 'the menu never rebuilds the widget from what its view was given');
    assert.deepEqual(result.after, { view: 'measure', measure: 'casualties', war: '7' }, `stored after the choice: ${JSON.stringify(result.stored)}`);
    assert.deepEqual(result.countries, { kind: 'tags', tags: ['GER'] });
    assert.equal(result.title, 'War contribution');
  }, 60_000));
});

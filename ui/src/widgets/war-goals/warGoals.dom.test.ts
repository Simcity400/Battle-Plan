import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('review 5 finding 11: a following war-goals widget that starts with nothing to show still asks for its pushed panel', async () => {
  await withNativePage('war-goals', '/ui/src/widgets/war-goals/warGoals.dom.fixture.tsx', page => page.stage('war goals', async () => {
    const cold = await page.evaluate<{ follows: string[]; text: string }>('window.warGoals.cold()');
    assert.deepEqual(cold.follows, ['wargoals'], `the loading state never asked: ${cold.text}`);
  }));
});

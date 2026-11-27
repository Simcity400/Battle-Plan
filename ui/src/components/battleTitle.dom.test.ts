import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('land and naval battle windows opened with a pending name take the arriving name without reopening', async () => {
  await withNativePage('battle-title', '/ui/src/components/battleTitle.dom.fixture.tsx', page => page.stage('battle titles', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.battleTitle.${expression}`);
    for (const kind of ['land', 'naval']) {
      assert.match(await act<string>(`mount('${kind}')`), /Loading/);
      assert.deepEqual(await act('resolveName()'), { title: 'Second Battle of Copper Hollow', sameWindow: true });
    }
  }));
});

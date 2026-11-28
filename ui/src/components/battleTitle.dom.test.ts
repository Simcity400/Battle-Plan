import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('land windows use the row title and naval windows take the arriving title without reopening', async () => {
  await withNativePage('battle-title', '/ui/src/components/battleTitle.dom.fixture.tsx', page => page.stage('battle titles', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.battleTitle.${expression}`);
    assert.equal(await act<string>("mount('land')"), 'Second Battle of Copper Hollow');
    assert.equal(await act<number>('nameRequests()'), 0);
    await act("mount('land', true)");
    assert.match(await act<string>('missingReason()'), /The province name was withheld/);
    assert.equal(await act<number>('nameRequests()'), 0);
    assert.match(await act<string>("mount('naval')"), /Loading/);
    assert.deepEqual(await act('resolveName()'), { title: 'Second Battle of Copper Hollow', sameWindow: true });
    await act("mount('naval', true)");
    await act('resolveName()');
    assert.match(await act<string>('missingReason()'), /The province name was withheld/);
  }));
});

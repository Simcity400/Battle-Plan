import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';



test('the panel reports a prompt seen only when the user acts inside it, never because it was drawn', async () => {
  await withNativePage('recorder-prompt', '/ui/src/recorder/RecorderPanel.dom.fixture.tsx', page => page.stage('recorder prompt', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.recorderPanelFixture.${expression}`);
    await act('render("main", 4)');
    assert.deepEqual(await act('seen()'), [], 'a Quit drawn in the panel isn’t one the user saw');
    await act('press(".recorder-head")');
    assert.deepEqual(await act('seen()'), [], 'a click elsewhere in the panel isn’t an act on the Quit');
    await act('press(".recorder-ask p")');
    assert.deepEqual(await act('seen()'), [4]);
    await act('render("main", 5, true)');
    await act('key(".recorder-prompt button")');
    assert.deepEqual(await act('seen()'), [4, 5], 'a key inside a prompt reissued for a new session reports that prompt');
  }, 60_000));
});


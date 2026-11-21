import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('the speed menu reopened takes a click at once: its presets are shown where they are the moment it opens', async () => {
  await withNativePage('playback-speed', '/ui/src/components/PlaybackSpeed.dom.fixture.tsx', page => page.stage('playback speed', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.playbackSpeed.${expression}`);
    assert.equal(await act('ready()'), true);
    assert.equal(await act('openAndClose()'), true, 'the first open draws the presets');
    const reopened = await act<{ open: boolean; visibility: string; hits: boolean[] }>('reopened()');
    assert.equal(reopened.open, true);
    assert.equal(reopened.visibility, 'visible', 'reopened, the menu is shown in the task that opened it');
    assert.ok(reopened.hits.length === 7 && reopened.hits.every(Boolean), `a click at each preset reaches it: ${JSON.stringify(reopened.hits)}`);
    assert.deepEqual(await act('pick(6)'), [1440], 'the preset clicked is the speed taken');
  }));
});

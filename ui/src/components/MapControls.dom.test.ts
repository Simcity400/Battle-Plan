import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('an export is asked for with the map settings closed, and a refusal reopens them with its reason', async () => {
  await withNativePage('map-controls', '/ui/src/components/MapControls.dom.fixture.tsx', page => page.stage('map controls export', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.mapControls.${expression}`);
    assert.equal(await act('ready()'), true);
    type Ended = { opened: boolean; asked: { view: string | null; menuOpen: boolean }[]; shown: boolean; said: string | null };
    const saved = await act<Ended>('exportWith(null)');
    assert.equal(saved.opened, true);
    assert.deepEqual(saved.asked, [{ view: 'widget-1', menuOpen: false }], 'asked once, with the menu closed');
    assert.equal(saved.shown, false, 'a saved export leaves the menu closed');
    const refused = await act<Ended>(`exportWith(${JSON.stringify('Capture map.png failed: its map was hidden before its picture was saved')})`);
    assert.deepEqual(refused.asked, [{ view: 'widget-1', menuOpen: false }]);
    assert.equal(refused.shown, true, 'a refusal opens the menu again');
    assert.equal(refused.said, 'Capture map.png failed: its map was hidden before its picture was saved');
  }));
});

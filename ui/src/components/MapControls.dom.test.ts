import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('an export is asked for with the map settings closed, and a refusal reopens them with what failed', async () => {
  await withNativePage('map-controls', '/ui/src/components/MapControls.dom.fixture.tsx', page => page.stage('map controls export', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.mapControls.${expression}`);
    assert.equal(await act('ready()'), true);
    type Ended = { opened: boolean; asked: { view: string | null; menuOpen: boolean }[]; shown: boolean; said: string | null; retry: string | null; note: string | null };
    const saved = await act<Ended>('exportWith(null)');
    assert.equal(saved.opened, true);
    assert.deepEqual(saved.asked, [{ view: 'widget-1', menuOpen: false }], 'asked once, with the menu closed');
    assert.equal(saved.shown, false, 'a saved export leaves the menu closed');
    const refused = await act<Ended>(`exportWith(${JSON.stringify('Couldn’t save the map image: Capture map.png failed: its map was hidden before its picture was saved')})`);
    assert.deepEqual(refused.asked, [{ view: 'widget-1', menuOpen: false }]);
    assert.equal(refused.shown, true, 'a refusal opens the menu again');
    assert.equal(refused.said, 'Couldn’t save the map image.', 'the technical reason stays out of the line');
    assert.equal(refused.retry, 'Try again', 'a failed export offers to export again');
    const warned = await act<Ended>(`exportWith(null, ${JSON.stringify('the change was made, but Windows didn’t confirm it reached the disk')})`);
    assert.equal(warned.shown, true, 'a save Windows didn’t confirm opens the menu to say so');
    assert.equal(warned.note, 'Saved map.png: branch 1, hour 12, but Windows didn’t confirm it reached the disk.');
    assert.equal(warned.retry, null, 'and offers no export again, since the picture was saved');
    const bare = await act<Ended>(`exportWith(${JSON.stringify('The map isn’t ready yet')})`);
    assert.equal(bare.said, 'Couldn’t export the map.', 'a raw fragment is detail; the line names what failed');
  }));
});

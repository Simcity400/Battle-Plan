import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('Raid tally and Raid log replace their paused views during playback and restore them on pause', async () => {
  await withNativePage('raid-playback', '/ui/src/widgets/raid-log/raidPlayback.dom.fixture.tsx', async page => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.raidPlayback.${expression}`);
    try {
      await act('mount()');
      assert.match(await act<string>('text()'), /prepared/);
      await act('playing(true)');
      assert.deepEqual(await act('states()'), ['Raid tally isn’t playing back yet.', 'Raid log isn’t playing back yet.']);
      assert.deepEqual(await act('advance()'), []);
      await act('playing(false)');
      assert.match(await act<string>('text()'), /prepared/);
    } finally { await act('close()'); }
  });
});

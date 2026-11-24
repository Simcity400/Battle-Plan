import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('Raid log replaces its paused view during playback and restores it on pause', async () => {
  await withNativePage('raid-playback', '/ui/src/widgets/raid-log/raidPlayback.dom.fixture.tsx', async page => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.raidPlayback.${expression}`);
    try {
      await act('mount()');
      assert.match(await act<string>('text()'), /prepared/);
      await act('playing(true)');
      assert.deepEqual(await act('states()'), ['Raid log isn’t playing back yet.']);
      assert.deepEqual(await act('advance()'), []);
      await act('playing(false)');
      assert.match(await act<string>('text()'), /prepared/);
    } finally { await act('close()'); }
  });
});

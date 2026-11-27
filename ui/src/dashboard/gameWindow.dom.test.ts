import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

type State = { hours: number[]; inFlight: number; image: string | null; error: string | null; retry: boolean };

test('game window cancellation', async t => {
  await withNativePage('game-window-cancellation', '/ui/src/dashboard/gameWindow.dom.fixture.tsx', async page => {
    const run = (call: string) => page.evaluate<State>(`window.gameWindow.${call}`);
    const scenario = (name: string, check: () => Promise<void>) => t.test(name, () => page.stage(name, async () => {
      await run('reset()');
      await check();
    }));

    await scenario('retries the latest cancelled request and shows the replacement', async () => {
      await run('open(100)');
      await run('show("Old frame")');
      await run('open(101)');
      const retried = await run('cancel()');
      assert.deepEqual(retried.hours, [100, 101, 101]);
      assert.equal(retried.image, 'equipment designer: Old frame');
      assert.equal(retried.retry, false);
      assert.equal((await run('show("Fresh frame")')).image, 'equipment designer: Fresh frame');
    });

    await scenario('stops after two consecutive cancellations and offers Retry', async () => {
      await run('open(100)');
      assert.deepEqual((await run('cancel()')).hours, [100, 100]);
      const stopped = await run('cancel()');
      assert.deepEqual(stopped.hours, [100, 100]);
      assert.equal(stopped.inFlight, 0);
      assert.match(stopped.error ?? '', /Couldn.t load equipment designer/);
      assert.equal(stopped.retry, true);
      assert.deepEqual((await run('retry()')).hours, [100, 100, 100]);
    });

    await scenario('replans after a cancellation when Follow changed at the same hour', async () => {
      await run('open(100)');
      assert.deepEqual((await run('open(100, 2)')).hours, [100]);
      const replanned = await run('cancel()');
      assert.deepEqual(replanned.hours, [100, 100]);
      assert.equal(replanned.inFlight, 1);
    });
  });
});

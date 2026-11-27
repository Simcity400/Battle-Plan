import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

type State = { hours: number[]; image: string | null; error: string | null; retry: boolean };

test('a game window retries once after cancellation and offers Retry after two cancellations', async () => {
  await withNativePage('game-window-cancellation', '/ui/src/dashboard/gameWindow.dom.fixture.tsx', page => page.stage('retry a game window', async () => {
    const run = (call: string) => page.evaluate<State>(`window.gameWindow.${call}`);
    await run('open(100)');
    await run('show("Old frame")');
    await run('open(101)');
    const retried = await run('cancel()');
    assert.deepEqual(retried.hours, [100, 101, 101]);
    assert.equal(retried.image, 'equipment designer: Old frame');
    assert.equal(retried.retry, false);
    assert.equal((await run('show("Fresh frame")')).image, 'equipment designer: Fresh frame');

    await run('open(102)');
    assert.deepEqual((await run('cancel()')).hours, [100, 101, 101, 102, 102]);
    const stopped = await run('cancel()');
    assert.deepEqual(stopped.hours, [100, 101, 101, 102, 102]);
    assert.match(stopped.error ?? '', /Couldn.t load equipment designer/);
    assert.equal(stopped.retry, true);
    assert.deepEqual((await run('retry()')).hours, [100, 101, 101, 102, 102, 102]);
  }));
});

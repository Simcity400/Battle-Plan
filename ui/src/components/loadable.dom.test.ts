import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('loadables share one load, show its code within frames of arriving, and load again after a failure', async () => {
  await withNativePage('loadable', '/ui/src/components/loadable.dom.fixture.tsx', page => page.stage('loadable', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.loadableFixture.${expression}`);
    await act('mount("Shown", ["a", "b"])');
    assert.equal(await act('text()'), 'loading aloading b');
    assert.equal(await act('loads("Shown")'), 1, 'instances share one load, under StrictMode too');
    // Revealed through Suspense, the code would wait until 300 ms after the fallback showed.
    await act('resolve("Shown")');
    assert.equal(await act('text()'), 'shown ashown b');
    const shownFallbacks = await act('fallbacks("Shown")');
    await act('mount("Shown", ["c"])');
    assert.equal(await act('text()'), 'shown c');
    assert.equal(await act('fallbacks("Shown")'), shownFallbacks, 'code already loaded renders without a fallback');
    assert.equal(await act('loads("Shown")'), 1);

    await act('mount("Flaky", ["d"])');
    await act('reject("Flaky")');
    assert.equal(await act('text()'), 'failed synthetic chunk failure', 'a failed load reaches the error boundary');
    await act('mount("Flaky", ["d"])');
    assert.equal(await act('loads("Flaky")'), 2, 'remounting, as Retry does, loads again');
    await act('resolve("Flaky")');
    assert.equal(await act('text()'), 'shown d');
  }, 60_000));
});

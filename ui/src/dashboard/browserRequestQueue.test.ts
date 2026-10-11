import assert from 'node:assert/strict';
import test from 'node:test';
import { BrowserRequestQueue } from './browserRequestQueue.ts';

const tick = () => new Promise<void>(resolve => setImmediate(resolve));
const deferred = <T>() => {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

test('long histories cannot take interactive or release connections, and queued histories all complete', async () => {
  const queue = new BrowserRequestQueue(), jobs = Array.from({ length: 5 }, () => deferred<number>()), starts: number[] = [];
  const histories = jobs.map((job, index) => queue.run('history', () => { starts.push(index); return job.promise; }));
  await tick();
  assert.deepEqual(starts, [0, 1, 2]);
  assert.equal(await queue.run('interactive', async () => 9), 9);
  assert.equal(await queue.run('control', async () => 10), 10);
  for (const [index, job] of jobs.entries()) { job.resolve(index); await tick(); }
  assert.deepEqual(await Promise.all(histories), [0, 1, 2, 3, 4]);
  assert.deepEqual(starts, [0, 1, 2, 3, 4]);
});

test('queued abort never runs and inflight abort holds admission until the operation settles', async () => {
  const queue = new BrowserRequestQueue(), jobs = Array.from({ length: 3 }, () => deferred<number>()), controllers = jobs.map(() => new AbortController());
  const signals: AbortSignal[] = [];
  const running = jobs.map((job, index) => queue.run('history', signal => { signals.push(signal); return job.promise; }, { signal: controllers[index].signal }));
  const queuedController = new AbortController();
  let queuedStarts = 0, nextStarts = 0;
  const queued = queue.run('history', async () => { queuedStarts++; return 7; }, { signal: queuedController.signal });
  const queuedFailure = assert.rejects(queued, { name: 'AbortError' });
  await tick();
  queuedController.abort();
  await queuedFailure;
  assert.equal(queuedStarts, 0);
  const next = queue.run('history', async () => { nextStarts++; return 8; });
  const activeFailure = assert.rejects(running[0], { name: 'AbortError' });
  controllers[0].abort();
  await tick();
  assert.equal(signals[0].aborted, true);
  assert.equal(nextStarts, 0, 'aborting does not admit another request over an occupied HTTP connection');
  jobs[0].resolve(0);
  await activeFailure;
  assert.equal(await next, 8);
  jobs[1].resolve(1); jobs[2].resolve(2);
  assert.deepEqual(await Promise.all(running.slice(1)), [1, 2]);
});

test('source cancellation isolates leases, queued sources and fresh retry ownership', async () => {
  const queue = new BrowserRequestQueue(), jobs = Array.from({ length: 3 }, () => deferred<number>());
  const started: string[] = [];
  const running = jobs.map((job, index) => queue.run('history', signal => {
    started.push(`${index}:${signal.aborted}`); return job.promise;
  }, { sourceId: index === 0 ? 'released' : 'other' }));
  let oldStarts = 0;
  const old = queue.run('history', async () => { oldStarts++; return 3; }, { sourceId: 'released' });
  const oldFailure = assert.rejects(old, { name: 'AbortError' }), activeFailure = assert.rejects(running[0], { name: 'AbortError' });
  await tick();
  queue.cancelSource('released');
  await oldFailure;
  assert.equal(oldStarts, 0);
  jobs[0].resolve(0); await activeFailure;
  assert.equal(await queue.run('history', async () => 4, { sourceId: 'fresh' }), 4);
  jobs[1].resolve(1); jobs[2].resolve(2);
  assert.deepEqual(await Promise.all(running.slice(1)), [1, 2]);
  assert.deepEqual(started, ['0:false', '1:false', '2:false']);
});

test('rejection frees only its lane and subsequent reads retain their own result', async () => {
  const queue = new BrowserRequestQueue(), first = deferred<string>(), second = deferred<string>();
  const rejected = queue.run('interactive', () => first.promise);
  const failure = assert.rejects(rejected, /Named refusal/);
  const independent = queue.run('interactive', () => second.promise);
  const retry = queue.run('interactive', async () => 'Retried title');
  first.reject(new Error('Named refusal'));
  assert.equal(await retry, 'Retried title');
  await failure;
  second.resolve('Independent title');
  assert.equal(await independent, 'Independent title');
});

test('a cancelled result is handed back to its owner for cleanup even when cancellation races operation resolution', async () => {
  const queue = new BrowserRequestQueue(), controller = new AbortController(), cleaned: string[] = [];
  const cancelled = queue.run('history', async () => {
    controller.abort();
    return 'late-lease';
  }, { signal: controller.signal, onCancelledValue: async lease => { cleaned.push(lease); } });
  await assert.rejects(cancelled, { name: 'AbortError' });
  assert.deepEqual(cleaned, ['late-lease']);
  assert.equal(await queue.run('history', async () => 'next-owner'), 'next-owner');
});

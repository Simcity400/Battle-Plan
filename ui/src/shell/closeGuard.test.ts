import assert from 'node:assert/strict';
import test from 'node:test';
import { type CloseGuardFailure, type CloseRequest, closeGuard, UNANSWERED_SAVED, UNANSWERED_UNSAVED, UNCLOSED, UNHEARD } from './closeGuard.ts';

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
const lost = 'BattlePlan couldn’t reach its desktop host: the IPC request failed (TypeError: Failed to fetch).';

function desktop() {
  const host = {
    unsaved: false,
    watchFails: false,
    answerFails: false,
    leaveFails: false,
    subscriptions: [] as ((request: CloseRequest) => void)[],
    watched: [] as number[],
    answers: [] as [number, boolean][],
    leaves: 0,
    prompts: 0,
    reports: [] as (readonly CloseGuardFailure[])[],
  };
  const watcher = new Map<number, (request: CloseRequest) => void>();
  const start = () => closeGuard({
    requests: onRequest => {
      const subscription = host.subscriptions.push(onRequest) - 1;
      return {
        watch: async () => {
          watcher.set(0, onRequest);
          host.watched.push(subscription);
          if (host.watchFails) throw lost;
        },
      };
    },
    answer: async (id, unsaved) => {
      host.answers.push([id, unsaved]);
      if (host.answerFails) throw lost;
    },
    leave: async () => {
      host.leaves++;
      if (host.leaveFails) throw 'Couldn’t close BattlePlan: no window';
    },
  }, { unsaved: () => host.unsaved, ask: () => { host.prompts++; } }, failures => { host.reports.push(failures); });
  const request = (request: CloseRequest) => watcher.get(0)?.(request);
  const shown = () => host.reports.at(-1) ?? [];
  return { host, start, request, shown };
}

test('a close request is judged by the page’s work at that moment', async () => {
  const { host, start, request } = desktop();
  start();
  await flush();
  request({ kind: 'close' });
  await flush();
  assert.deepEqual([host.leaves, host.prompts], [1, 0], 'saved work closes');
  host.unsaved = true;
  request({ kind: 'close' });
  assert.deepEqual([host.leaves, host.prompts], [1, 1], 'unsaved work asks');
});

test('a close while a save’s reply is held and a new edit is queued asks about the new edit', async () => {
  const { host, start, request } = desktop();
  start();
  host.unsaved = true;
  host.unsaved = false;
  host.unsaved = true;
  request({ kind: 'close' });
  await flush();
  assert.deepEqual([host.leaves, host.prompts], [0, 1], 'nothing the host heard earlier decides the close');
});

test('a question the host asks is answered from the page’s work at that moment, by its id', async () => {
  const { host, start, request } = desktop();
  start();
  request({ kind: 'ask', id: 4 });
  host.unsaved = true;
  request({ kind: 'ask', id: 5 });
  await flush();
  assert.deepEqual(host.answers, [[4, false], [5, true]]);
  assert.deepEqual([host.leaves, host.prompts], [0, 0], 'answering never closes or prompts');
});

test('a registration whose reply was lost is sent again on Try again, with the same subscription', async () => {
  const { host, start, request, shown } = desktop();
  host.watchFails = true;
  start();
  await flush();
  assert.deepEqual(shown().map(({ message, detail }) => [message, detail]), [[UNHEARD, lost]]);
  host.watchFails = false;
  shown()[0]!.retry();
  shown()[0]!.retry();
  await flush();
  assert.deepEqual(host.watched, [0, 0], 'one subscription, registered again once');
  assert.equal(host.subscriptions.length, 1);
  assert.deepEqual(shown(), []);
  host.unsaved = true;
  request({ kind: 'close' });
  assert.equal(host.prompts, 1, 'a close is heard once');
});

test('a superseded guard’s subscription is inert', async () => {
  const { host, start } = desktop();
  const first = start();
  first.end();
  start();
  await flush();
  host.unsaved = true;
  host.subscriptions[0]!({ kind: 'close' });
  host.subscriptions[0]!({ kind: 'ask', id: 1 });
  assert.deepEqual([host.prompts, host.answers.length, host.leaves], [0, 0, 0]);
  host.subscriptions[1]!({ kind: 'close' });
  assert.equal(host.prompts, 1);
});

test('each failure is shown by name with its own retry, and no retry replays another failure’s work', async () => {
  const { host, start, request, shown } = desktop();
  start();
  await flush();
  host.answerFails = true;
  host.leaveFails = true;
  request({ kind: 'ask', id: 7 });
  request({ kind: 'close' });
  await flush();
  assert.deepEqual(shown().map(({ message }) => message), [UNANSWERED_SAVED, UNCLOSED]);
  const [unanswered, unclosed] = shown();
  host.answerFails = false;
  host.unsaved = true;
  unanswered!.retry();
  await flush();
  assert.deepEqual(host.answers, [[7, false], [7, true]], 'the answer is sent again from the work now');
  assert.equal(host.leaves, 1, 'retrying the answer doesn’t close BattlePlan');
  assert.deepEqual(shown().map(({ message }) => message), [UNCLOSED]);

  unclosed!.retry();
  await flush();
  assert.deepEqual([host.leaves, host.prompts], [1, 1], 'retrying the close judges the work again, and now asks');
  assert.deepEqual(shown(), []);
  unclosed!.retry();
  assert.deepEqual([host.leaves, host.prompts], [1, 1], 'a retry is bound to the failure it was shown with');
});

test('an answer that fails says which way it failed', async () => {
  const { host, start, request, shown } = desktop();
  start();
  host.answerFails = true;
  host.unsaved = true;
  request({ kind: 'ask', id: 2 });
  await flush();
  assert.deepEqual(shown().map(({ message, detail }) => [message, detail]), [[UNANSWERED_UNSAVED, lost]]);
});

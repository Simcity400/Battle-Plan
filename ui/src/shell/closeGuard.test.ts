import assert from 'node:assert/strict';
import test from 'node:test';
import { type CloseAbout, type CloseGuardFailure, type CloseRequest, closeGuard, type LeavingFor, UNANSWERED_SAVED, UNANSWERED_UNSAVED, UNCLOSED, UNHEARD, UNKEPT } from './closeGuard.ts';
import { keepLatest } from './latest.ts';
import { unsavedDrafts } from '../dashboard/layoutDrafts.ts';
import { loadLayouts } from '../dashboard/storage.ts';
import type { PageId } from '../dashboard/pages';
import '../widgets/definitions.ts';

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
const lost = 'BattlePlan couldn’t reach its desktop host: the IPC request failed (TypeError: Failed to fetch).';

function desktop(judge?: () => boolean) {
  const host = {
    unsaved: false,
    confirmed: false,
    heard: true,
    watchFails: false,
    answerFails: false,
    leaveFails: false,
    subscriptions: [] as ((request: CloseRequest) => void)[],
    watched: [] as number[],
    answers: [] as [number, boolean][],
    leaves: 0,
    prompts: 0,
    leaving: [] as (LeavingFor | null)[],
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
    answer: async (id, stays) => {
      host.answers.push([id, stays]);
      if (host.answerFails) throw lost;
      return host.heard;
    },
    leave: async () => {
      host.leaves++;
      if (host.leaveFails) throw 'Couldn’t close BattlePlan: no window';
    },
  }, {
    unsaved: judge ?? (() => host.unsaved),
    confirmed: () => host.confirmed,
    prompt: () => { host.prompts++; },
    leaving: leaving => { host.leaving.push(leaving); },
  }, failures => { host.reports.push(failures); });
  const ask = (id: number, about: CloseAbout) => watcher.get(0)?.({ kind: 'ask', id, about });
  const stay = (id: number) => watcher.get(0)?.({ kind: 'stay', id });
  const shown = () => host.reports.at(-1) ?? [];
  const locked = () => host.leaving.at(-1) ?? null;
  return { host, start, ask, stay, shown, locked };
}

test('a close is judged by the page’s work at that moment, and saved work locks editing before it leaves', async () => {
  const { host, start, ask, locked } = desktop();
  start();
  await flush();
  host.unsaved = true;
  ask(1, 'close');
  assert.deepEqual([host.prompts, host.answers.at(-1), locked()], [1, [1, true], null], 'unsaved work prompts and stays');
  host.unsaved = false;
  ask(2, 'close');
  assert.deepEqual([host.prompts, host.answers.at(-1), locked()], [1, [2, false], 'close'], 'saved work leaves, locked first');
  host.unsaved = true;
  ask(3, 'close');
  assert.deepEqual([host.prompts, host.answers.at(-1)], [1, [3, false]], 'a page already leaving isn’t asked to keep an edit it can’t make');
});

test('a close between a layout edit and the dashboard’s report of it asks about the edit', async () => {
  const { library } = loadLayouts({ getItem: () => null, setItem: () => {} }, 'overview');
  const saved = library.layouts[0].template;
  const drafts = { page: { current: 'overview' as PageId }, library: { current: library }, template: { current: saved },
    kept: { current: new Map() }, pages: { current: ['overview' as PageId] }, campaigns: { current: [] } };
  const drawn: unknown[] = [];
  const setTemplate = keepLatest(drafts.template, template => { drawn.push(template); });
  const { host, start, ask, locked } = desktop(() => unsavedDrafts(drafts).length > 0);
  start();
  await flush();
  setTemplate(previous => ({ ...previous, widgets: previous.widgets.slice(1) }));
  ask(1, 'close');
  assert.deepEqual([host.prompts, host.answers, locked()], [1, [[1, true]], null], 'the edit holds the window before the shell hears of it');
  assert.equal(drawn.length, 1, 'the guard read the edit itself, not a render or a report of it');
});

test('a question the host asks is answered from the page’s work at that moment, by its id, and a busy query locks nothing', async () => {
  const { host, start, ask, locked } = desktop();
  start();
  ask(4, 'query');
  host.unsaved = true;
  ask(5, 'query');
  ask(6, 'leave');
  await flush();
  assert.deepEqual(host.answers, [[4, false], [5, true], [6, true]]);
  assert.deepEqual([host.leaves, host.prompts, locked()], [0, 0, null], 'answering never closes, prompts or locks editing');
});

test('Discard and restart lets the update leave only while its confirmation stands, and editing stays locked until the host says stay', async () => {
  const { host, start, ask, stay, locked } = desktop();
  start();
  await flush();
  host.unsaved = true;
  host.confirmed = true;
  host.confirmed = false;
  ask(1, 'leave');
  assert.deepEqual([host.answers.at(-1), locked()], [[1, true], null], 'a confirmation cancelled before the update asked authorises nothing');
  host.confirmed = true;
  ask(2, 'leave');
  assert.deepEqual([host.answers.at(-1), locked()], [[2, false], 'update'], 'the confirmed restart leaves, and editing locks before it answers');
  host.confirmed = false;
  ask(3, 'leave');
  assert.deepEqual(host.answers.at(-1), [3, false], 'once leaving, the page doesn’t take it back');
  stay(2);
  assert.equal(locked(), 'update', 'another update still holds the lock');
  stay(3);
  assert.equal(locked(), null, 'an install that couldn’t start lets the page go on');
  stay(3);
  assert.deepEqual(host.leaving, ['update', null], 'a stay sent again changes nothing');
});

test('an answer the host no longer waits for lets the page go on', async () => {
  const { host, start, ask, locked } = desktop();
  start();
  host.heard = false;
  ask(1, 'leave');
  assert.equal(locked(), 'update');
  await flush();
  assert.equal(locked(), null, 'the question was refused before the answer arrived, so nothing leaves on it');
});

test('Discard and close locks editing before it leaves, and a failed leave keeps it locked with its own Try again', async () => {
  const { host, start, ask, shown, locked } = desktop();
  const guard = start();
  await flush();
  host.unsaved = true;
  host.leaveFails = true;
  guard.leave();
  assert.deepEqual([locked(), host.leaves], ['close', 1]);
  await flush();
  assert.deepEqual(shown().map(({ message }) => message), [UNCLOSED]);
  assert.equal(locked(), 'close', 'a lost reply may hide a close the host made, so editing stays locked');
  ask(1, 'close');
  assert.deepEqual([host.answers.at(-1), host.prompts], [[1, false], 0], 'the window’s close button closes it');
  host.leaveFails = false;
  shown()[0]!.retry();
  await flush();
  assert.deepEqual([host.leaves, shown()], [2, []]);
});

test('a registration whose reply was lost is sent again on Try again, with the same subscription', async () => {
  const { host, start, ask, shown } = desktop();
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
  ask(1, 'close');
  assert.equal(host.prompts, 1, 'a close is heard once');
});

test('a superseded guard’s subscription is inert', async () => {
  const { host, start } = desktop();
  const first = start();
  first.end();
  start();
  await flush();
  host.unsaved = true;
  host.subscriptions[0]!({ kind: 'ask', id: 1, about: 'close' });
  host.subscriptions[0]!({ kind: 'ask', id: 2, about: 'query' });
  assert.deepEqual([host.prompts, host.answers.length, host.leaves], [0, 0, 0]);
  host.subscriptions[1]!({ kind: 'ask', id: 3, about: 'close' });
  assert.equal(host.prompts, 1);
});

test('each failure is shown by name with its own retry, and no retry replays another failure’s work', async () => {
  const { host, start, ask, shown } = desktop();
  const guard = start();
  await flush();
  host.answerFails = true;
  host.leaveFails = true;
  ask(7, 'query');
  guard.leave();
  await flush();
  assert.deepEqual(shown().map(({ message }) => message), [UNANSWERED_SAVED, UNCLOSED]);
  const [unanswered, unclosed] = shown();
  host.answerFails = false;
  unanswered!.retry();
  await flush();
  assert.deepEqual(host.answers, [[7, false], [7, false]], 'the answer is sent again from the page as it is now');
  assert.equal(host.leaves, 1, 'retrying the answer doesn’t close BattlePlan');
  assert.deepEqual(shown().map(({ message }) => message), [UNCLOSED]);
  host.leaveFails = false;
  unclosed!.retry();
  await flush();
  assert.deepEqual([host.leaves, shown()], [2, []]);
  unclosed!.retry();
  assert.equal(host.leaves, 2, 'a retry is bound to the failure it was shown with');
});

test('an answer that fails says which way it failed', async () => {
  const { host, start, ask, shown } = desktop();
  start();
  host.answerFails = true;
  host.unsaved = true;
  ask(2, 'leave');
  ask(3, 'close');
  await flush();
  assert.deepEqual(shown().map(({ message, detail }) => [message, detail]), [[UNANSWERED_UNSAVED, lost], [UNKEPT, lost]]);
});

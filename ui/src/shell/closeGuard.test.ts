import assert from 'node:assert/strict';
import test from 'node:test';
import { type CloseAbout, type CloseAsk, type CloseBatch, type CloseGuardFailure, closeGuard, type LeavingFor, UNANSWERED_SAVED, UNANSWERED_UNSAVED, UNCLOSED, UNHEARD, UNKEPT } from './closeGuard.ts';
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
    subscriptions: [] as ((batch: CloseBatch) => boolean)[],
    waiting: new Set<number>(),
    holds: new Map<number, CloseAbout>(),
    watched: [] as number[],
    answers: [] as [number, boolean][],
    bases: [] as (readonly number[])[],
    leaves: 0,
    prompts: 0,
    leaving: [] as (LeavingFor | null)[],
    reports: [] as (readonly CloseGuardFailure[])[],
  };
  const watcher = new Map<number, (batch: CloseBatch) => boolean>();
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
    answer: async (id, stays, basis) => {
      host.answers.push([id, stays]);
      host.bases.push(basis);
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
  const batch = (...asks: CloseAsk[]): CloseBatch => {
    for (const { id } of asks) host.waiting.add(id);
    return { asks, waiting: [...host.waiting], holds: [...host.holds].map(([id, about]) => ({ id, about })) };
  };
  const send = (...asks: CloseAsk[]) => watcher.get(0)?.(batch(...asks));
  const ask = (id: number, about: CloseAbout) => send({ id, about });
  const hold = (id: number, about: CloseAbout) => { host.waiting.delete(id); host.holds.set(id, about); };
  const retire = (...ids: number[]) => { for (const id of ids) { host.waiting.delete(id); host.holds.delete(id); } };
  const stay = (id: number) => { retire(id); send(); };
  const shown = () => host.reports.at(-1) ?? [];
  const locked = () => host.leaving.at(-1) ?? null;
  return { host, start, batch, send, ask, hold, retire, stay, shown, locked };
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
  const { host, start, ask, hold, stay, locked } = desktop();
  start();
  await flush();
  host.unsaved = true;
  host.confirmed = true;
  host.confirmed = false;
  ask(1, 'install');
  assert.deepEqual([host.answers.at(-1), locked()], [[1, true], null], 'a confirmation cancelled before the update asked authorises nothing');
  host.confirmed = true;
  ask(4, 'leave');
  assert.deepEqual([host.answers.at(-1), locked()], [[4, true], null], 'a setup’s close isn’t the restart the user agreed to');
  ask(2, 'install');
  assert.deepEqual([host.answers.at(-1), host.bases.at(-1), locked()], [[2, false], [], 'update'],
    'the confirmed restart leaves on the user’s word alone, and editing locks before it answers');
  hold(2, 'install');
  host.confirmed = false;
  ask(3, 'leave');
  assert.deepEqual([host.answers.at(-1), host.bases.at(-1)], [[3, false], [2]], 'once leaving, the page doesn’t take it back, and says why');
  hold(3, 'leave');
  stay(2);
  assert.equal(locked(), 'update', 'another update still holds the lock');
  stay(3);
  assert.equal(locked(), null, 'an install that couldn’t start lets the page go on');
  stay(3);
  assert.deepEqual(host.leaving, [null, 'update', null], 'a stay sent again changes nothing');
});

test('a batch that retires a leave and asks a close is taken in order, so the close is judged after the page stops leaving', async () => {
  const { host, start, send, ask, hold, retire, locked } = desktop();
  start();
  await flush();
  host.unsaved = true;
  host.confirmed = true;
  ask(1, 'install');
  hold(1, 'install');
  assert.equal(locked(), 'update');
  host.confirmed = false;
  retire(1);
  send({ id: 2, about: 'close' });
  assert.deepEqual([host.answers.at(-1), host.prompts, locked()], [[2, true], 1, null], 'the failed restart no longer speaks for the unsaved work');
});

test('a close judged while an install stood is retired with it, so the close asked again is judged afresh', async () => {
  const { host, start, send, ask, hold, retire, locked } = desktop();
  start();
  await flush();
  host.unsaved = true;
  host.confirmed = true;
  ask(1, 'install');
  hold(1, 'install');
  ask(2, 'close');
  assert.deepEqual([host.answers.at(-1), host.bases.at(-1), locked()], [[2, false], [1], 'update'],
    'a close while restarting goes with the restart, and names it, so the host can refuse it if the restart no longer stands');
  host.confirmed = false;
  retire(1, 2);
  send({ id: 3, about: 'close' });
  assert.deepEqual([host.answers.at(-1), host.prompts, locked()], [[3, true], 1, null],
    'the installer couldn’t start, so neither the restart nor the close judged under it speaks for the unsaved work');
});

test('a page that loads while BattlePlan holds a leave is locked by the first batch it hears', async () => {
  const { host, start, send, hold, locked } = desktop();
  hold(9, 'install');
  start();
  await flush();
  host.unsaved = true;
  assert.equal(send(), true);
  assert.equal(locked(), 'update');
});

test('a leave whose answer was lost stays locked until the host says stay, and the stay clears that answer’s Try again', async () => {
  const { host, start, ask, hold, stay, shown, locked } = desktop();
  start();
  await flush();
  host.answerFails = true;
  ask(4, 'leave');
  await flush();
  assert.deepEqual([shown().map(({ message }) => message), locked()], [[UNANSWERED_SAVED], 'update'], 'the host may have heard it, so editing stays locked');
  host.answerFails = false;
  shown()[0]!.retry();
  await flush();
  assert.deepEqual([host.answers.at(-1), locked(), shown()], [[4, false], 'update', []], 'the same leave is sent again, and the host still holds it');
  hold(4, 'leave');
  host.answerFails = true;
  ask(5, 'leave');
  await flush();
  stay(5);
  assert.deepEqual([locked(), shown()], ['update', []], 'an install that couldn’t start leaves nothing of its question to retry');
  stay(4);
  assert.equal(locked(), null);
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

test('a guard that has ended reports that it handled nothing, so the host never counts its batch as heard', async () => {
  const { host, start, batch, hold, locked } = desktop();
  const first = start();
  first.end();
  start();
  await flush();
  host.unsaved = true;
  assert.equal(host.subscriptions[0]!(batch({ id: 1, about: 'close' }, { id: 2, about: 'query' })), false);
  assert.deepEqual([host.prompts, host.answers.length, host.leaves, host.leaving], [0, 0, 0, []]);
  hold(5, 'leave');
  assert.equal(host.subscriptions[1]!(batch({ id: 3, about: 'close' })), true);
  assert.deepEqual([host.answers, locked()], [[[3, false]], 'update'], 'the guard that replaced it takes the leave the host holds');
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
  assert.deepEqual(host.answers, [[7, false], [7, false]], 'the same answer is sent again');
  assert.equal(host.leaves, 1, 'retrying the answer doesn’t close BattlePlan');
  assert.deepEqual(shown().map(({ message }) => message), [UNCLOSED]);
  host.leaveFails = false;
  unclosed!.retry();
  await flush();
  assert.deepEqual([host.leaves, shown()], [2, []]);
  unclosed!.retry();
  assert.equal(host.leaves, 2, 'a retry is bound to the failure it was shown with');
});

test('Try again sends the answer the page gave, not a new judgment made while it was leaving', async () => {
  const { host, start, ask, hold, shown } = desktop();
  start();
  await flush();
  host.unsaved = true;
  host.confirmed = true;
  ask(1, 'install');
  hold(1, 'install');
  host.answerFails = true;
  ask(2, 'close');
  await flush();
  host.answerFails = false;
  host.confirmed = false;
  shown()[0]!.retry();
  await flush();
  assert.deepEqual([host.answers.slice(-2), host.bases.slice(-2)], [[[2, false], [2, false]], [[1], [1]]],
    'the retry rests on the restart, as the answer did, and never on the close’s own lock');
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

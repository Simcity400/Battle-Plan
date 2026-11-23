import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { type CloseAbout, type CloseAsk, type CloseBatch, type CloseGuard, type CloseGuardFailure, closeGuard, type LeavingFor, UNANSWERED_SAVED, UNANSWERED_UNSAVED, UNCLOSED, UNHEARD, UNJUDGED, UNKEPT, UNRECHECKED } from './closeGuard.ts';
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
    refreshFails: false,
    refreshes: 0,
    installing: false,
    holdRefreshes: false,
    heldRefreshes: [] as ((seq: number) => void)[],
    subscriptions: [] as ((batch: CloseBatch) => boolean)[],
    seq: 0,
    waiting: new Map<number, CloseAbout>(),
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
        refresh: async () => {
          host.refreshes++;
          if (host.refreshFails) throw lost;
          if (host.holdRefreshes) return new Promise<number>(resolve => { host.heldRefreshes.push(resolve); });
          return host.seq + 1;
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
    for (const { id, about } of asks) host.waiting.set(id, about);
    const listed = (asked: Map<number, CloseAbout>) => [...asked].map(([id, about]) => ({ id, about }));
    return { seq: ++host.seq, waiting: listed(host.waiting), holds: listed(host.holds), installing: host.installing };
  };
  const deliver = (sent: CloseBatch) => watcher.get(0)?.(sent);
  const send = (...asks: CloseAsk[]) => deliver(batch(...asks));
  const ask = (id: number, about: CloseAbout) => send({ id, about });
  const hold = (id: number, about: CloseAbout) => { host.waiting.delete(id); host.holds.set(id, about); };
  const retire = (...ids: number[]) => { for (const id of ids) { host.waiting.delete(id); host.holds.delete(id); } };
  const stay = (id: number) => { retire(id); send(); };
  const shown = () => host.reports.at(-1) ?? [];
  const locked = () => host.leaving.at(-1) ?? null;
  return { host, start, batch, deliver, send, ask, hold, retire, stay, shown, locked };
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
  await flush();
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

test('a restart whose reply is lost before its install question reaches the page keeps Try again on the lock that question takes', async () => {
  const { host, start, ask, retire, batch, deliver, shown, locked } = desktop();
  const guard = start();
  await flush();
  guard.recheck();
  await flush();
  assert.deepEqual([locked(), shown()], [null, []], 'a clean page stays editable and shows nothing while the install is still on its way');
  ask(7, 'install');
  await flush();
  assert.deepEqual([locked(), host.answers.at(-1)], ['update', [7, false]]);
  assert.equal(shown().find(failure => failure.key === 'recheck')?.message, UNRECHECKED, 'the lock the late question takes still offers Try again');
  retire(7);
  batch();
  shown().find(failure => failure.key === 'recheck')!.retry();
  await flush();
  deliver(batch());
  await flush();
  assert.deepEqual([locked(), shown()], [null, []], 'so a launch that failed and whose stay was lost never strands Restarting…');
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

test('Discard and restart lets the update leave only while its confirmation stands, and editing stays locked while a leave is held', async () => {
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
  assert.equal(locked(), 'update', 'setup’s accepted close commits BattlePlan to closing, so a failed install leaves editing locked');
  stay(2);
  assert.deepEqual(host.leaving, [null, 'update'], 'a stay sent again changes nothing');
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

test('a leave that would rest on an answer the host hasn’t taken waits for it, so a lost answer never becomes consent', async () => {
  const { host, start, send, ask, hold, stay, shown, locked } = desktop();
  start();
  await flush();
  host.unsaved = true;
  host.confirmed = true;
  ask(1, 'install');
  await flush();
  hold(1, 'install');
  host.answerFails = true;
  ask(2, 'close');
  await flush();
  assert.deepEqual([host.answers.at(-1), host.bases.at(-1), locked()], [[2, false], [1], 'update'], 'a close while restarting goes with the restart');
  host.confirmed = false;
  stay(1);
  assert.equal(locked(), 'close', 'the close the page agreed to may have reached the host, so editing stays locked');
  ask(3, 'leave');
  assert.equal(host.answers.length, 2, 'setup’s close isn’t judged on a close the host may never have taken');
  host.answerFails = false;
  host.heard = false;
  shown()[0]!.retry();
  send();
  await flush();
  assert.deepEqual([host.answers.slice(-2), locked()], [[[2, false], [3, true]], null],
    'the host refuses the close, which rested on the failed restart, and setup’s close is judged afresh');
});

test('a batch older than one the page has applied changes nothing, so a crossing batch can’t unlock a page whose answer is in flight', async () => {
  const { host, start, batch, deliver, locked } = desktop();
  start();
  await flush();
  const older = batch({ id: 1, about: 'query' });
  const newer = batch({ id: 2, about: 'close' });
  assert.equal(deliver(newer), true);
  assert.deepEqual([host.answers, locked()], [[[1, false], [2, false]], 'close'], 'the newer batch carries the query the older one asked');
  assert.equal(deliver(older), true, 'the older batch is covered by the newer one');
  assert.deepEqual([host.answers.length, locked()], [2, 'close']);
});

test('a batch the page couldn’t finish judging is reported unhandled, Try again asks the host what stands, and an older batch judges only what it carries', async () => {
  let readable = false;
  const { host, start, batch, deliver, send, shown } = desktop(() => { if (!readable) throw new Error('the drafts couldn’t be read'); return false; });
  start();
  await flush();
  const older = batch({ id: 9, about: 'install' });
  const newer = batch();
  assert.equal(deliver(newer), false, 'the install wasn’t judged, so the host doesn’t count it heard');
  assert.equal(deliver(older), false, 'nor the older batch that carries it');
  assert.deepEqual([host.answers, shown().map(({ message }) => message)], [[], [UNJUDGED]], 'the failure is shown by name');
  readable = true;
  shown()[0]!.retry();
  assert.deepEqual([host.answers, host.refreshes, shown().map(({ message }) => message)], [[], 1, [UNJUDGED]],
    'Try again judges nothing from what the page last heard, which the host may have retired since');
  assert.equal(send(), true, 'the batch the host sends judges what stands now');
  assert.deepEqual([host.answers, shown()], [[[9, false]], []]);
  assert.equal(deliver(older), true, 'a batch the page has now judged in full is covered');
  readable = false;
  const carrying = batch({ id: 10, about: 'query' });
  const later = batch({ id: 11, about: 'query' });
  assert.equal(deliver(later), false);
  readable = true;
  assert.equal(deliver(older), true, 'an older batch answers nothing it doesn’t carry');
  assert.equal(host.answers.length, 1);
  assert.equal(deliver(carrying), true, 'an older batch carrying a question the newer one still lists judges it');
  assert.deepEqual([host.answers.slice(1), shown().map(({ message }) => message)], [[[10, false]], [UNJUDGED]], 'the question it doesn’t carry waits for Try again');
});

test('Try again after a batch the page couldn’t judge never answers a close the host retired meanwhile', async () => {
  let readable = false;
  const { host, start, ask, send, retire, shown } = desktop(() => { if (!readable) throw new Error('the drafts couldn’t be read'); return false; });
  start();
  await flush();
  assert.equal(ask(1, 'close'), false, 'the only batch carrying the close couldn’t be judged');
  retire(1);
  readable = true;
  host.refreshFails = true;
  shown()[0]!.retry();
  await flush();
  assert.deepEqual([host.answers, host.refreshes, shown().map(({ message, detail }) => [message, detail])], [[], 1, [[UNJUDGED, lost]]],
    'the close the host no longer waits on isn’t answered, and a lost request keeps the failure with its Try again');
  host.refreshFails = false;
  shown()[0]!.retry();
  assert.equal(host.refreshes, 2);
  send({ id: 2, about: 'close' });
  assert.deepEqual([host.answers, shown()], [[[2, false]], []], 'the close asked again with a fresh id is judged and answered');
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

test('a leave whose answer was lost stays locked until the host gives it up, and that clears its Try again', async () => {
  const { host, start, send, ask, hold, stay, shown, locked } = desktop();
  start();
  await flush();
  host.answerFails = true;
  ask(4, 'leave');
  await flush();
  assert.deepEqual([shown().map(({ message }) => message), locked()], [[UNANSWERED_SAVED], 'update'], 'the host may have heard it, so editing stays locked');
  host.answerFails = false;
  shown()[0]!.retry();
  assert.equal(host.answers.length, 1, 'Try again first asks the host whether the question still stands');
  send();
  await flush();
  assert.deepEqual([host.answers.at(-1), locked(), shown()], [[4, false], 'update', []], 'it does, so the same leave is sent again, and the host still holds it');
  hold(4, 'leave');
  host.answerFails = true;
  ask(5, 'leave');
  await flush();
  stay(5);
  assert.deepEqual([locked(), shown()], ['update', []], 'a question the host gave up leaves nothing to retry, and the close it accepted keeps editing locked');
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
  const { host, start, batch, hold, retire, locked } = desktop();
  const first = start();
  first.end();
  start();
  await flush();
  host.unsaved = true;
  assert.equal(host.subscriptions[0]!(batch({ id: 1, about: 'close' }, { id: 2, about: 'query' })), false);
  assert.deepEqual([host.prompts, host.answers.length, host.leaves, host.leaving], [0, 0, 0, []]);
  retire(1, 2);
  hold(5, 'leave');
  assert.equal(host.subscriptions[1]!(batch({ id: 3, about: 'close' })), true);
  assert.deepEqual([host.answers, locked()], [[[3, false]], 'update'], 'the guard that replaced it takes the leave the host holds');
});

test('each failure is shown by name with its own retry, and no retry replays another failure’s work', async () => {
  const { host, start, send, ask, shown } = desktop();
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
  send();
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
  const { host, start, send, ask, hold, shown } = desktop();
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
  send();
  await flush();
  assert.deepEqual([host.answers.slice(-2), host.bases.slice(-2)], [[[2, false], [2, false]], [[1], [1]]],
    'the retry rests on the restart, as the answer did, and never on the close’s own lock');
});

test('Try again never resends an answer whose question the host has retired since', async () => {
  const { host, start, ask, send, retire, shown } = desktop();
  start();
  await flush();
  host.answerFails = true;
  ask(4, 'leave');
  await flush();
  host.answerFails = false;
  shown()[0]!.retry();
  retire(4);
  send();
  await flush();
  assert.deepEqual([host.answers, shown()], [[[4, false]], []], 'the batch shows the question gone, so its answer and its failure go with it');
});

test('Try again resends only on the batch its own request brought, and only while that batch still waits on the answer', async () => {
  const { host, start, batch, deliver, send, ask, hold, shown, locked } = desktop();
  start();
  await flush();
  host.answerFails = true;
  ask(4, 'leave');
  await flush();
  assert.deepEqual([host.answers, shown().map(({ message }) => message), locked()], [[[4, false]], [UNANSWERED_SAVED], 'update']);
  host.answerFails = false;
  const queued = batch();
  shown()[0]!.retry();
  await flush();
  hold(4, 'leave');
  assert.equal(deliver(queued), true);
  assert.equal(host.answers.length, 1, 'a batch the host queued before the request isn’t its answer');
  send();
  await flush();
  assert.deepEqual([host.answers.length, shown(), locked()], [1, [], 'update'], 'the host took the answer whose reply was lost, so nothing is sent again and nothing stays to retry');
});

test('a second Try again while the first is still asking does nothing, so an answer the host took is never sent again', async () => {
  const { host, start, send, ask, hold, shown, locked } = desktop();
  start();
  await flush();
  host.answerFails = true;
  ask(4, 'leave');
  await flush();
  host.holdRefreshes = true;
  shown()[0]!.retry();
  shown()[0]!.retry();
  assert.equal(host.refreshes, 1, 'one request for what stands per failure');
  send();
  const seq = host.seq;
  host.heldRefreshes.shift()!(seq);
  await flush();
  assert.equal(host.answers.length, 2, 'the answer is sent again once');
  hold(4, 'leave');
  await flush();
  assert.equal(shown().length, 1, 'its reply was lost too, so it fails again');
  for (const reply of host.heldRefreshes.splice(0)) reply(seq);
  await flush();
  assert.deepEqual([host.answers.length, locked()], [2, 'update'], 'nothing an earlier failure asked for resends it');
  shown()[0]!.retry();
  host.heldRefreshes.shift()!(seq + 1);
  send();
  await flush();
  assert.deepEqual([host.answers.length, shown()], [2, []], 'the batch says the host holds it, so the failure goes and nothing is sent');
});

test('an install that couldn’t start asks the host what stands, so a lost stay never leaves Restarting… up with nothing to try', async () => {
  const { host, start, send, ask, hold, retire, shown, locked } = desktop();
  const guard = start();
  await flush();
  ask(1, 'install');
  hold(1, 'install');
  send();
  assert.equal(locked(), 'update');
  retire(1);
  host.refreshFails = true;
  guard.recheck();
  await flush();
  assert.deepEqual([host.refreshes, shown().map(({ message }) => message), locked()], [1, [UNRECHECKED], 'update'], 'the modal shows that it couldn’t ask, with Try again');
  host.refreshFails = false;
  shown()[0]!.retry();
  await flush();
  assert.equal(host.refreshes, 2);
  send();
  assert.deepEqual([locked(), shown()], [null, []], 'the batch the request brought unlocks the page');
  guard.recheck();
  await flush();
  assert.deepEqual([host.refreshes, locked(), shown()], [3, null, []], 'a page that holds nothing still asks, since the install it asked for may be on its way, and shows nothing while it is editable');
});

test('a recheck the host dispatched keeps its Try again until the page runs a batch at least as new as the one it named', async () => {
  const { host, start, deliver, send, ask, hold, retire, shown, locked } = desktop();
  const guard = start();
  await flush();
  ask(1, 'install');
  hold(1, 'install');
  send();
  retire(1);
  const queued = ++host.seq;
  guard.recheck();
  await flush();
  assert.deepEqual([host.refreshes, shown().map(({ message }) => message), locked()], [1, [UNRECHECKED], 'update'], 'the host dispatched its batch, but the page hasn’t run it, so Restarting… keeps Try again');
  deliver({ seq: queued, waiting: [], holds: [{ id: 1, about: 'install' }], installing: false });
  assert.deepEqual([shown().map(({ message }) => message), locked()], [[UNRECHECKED], 'update'], 'a batch older than the one named doesn’t settle it');
  shown()[0]!.retry();
  await flush();
  assert.equal(host.refreshes, 2, 'Try again asks again');
  send();
  assert.deepEqual([locked(), shown()], [null, []], 'the batch the request named unlocks the page');
});

test('a restart whose reply is lost while its installer still starts keeps Try again until the install is resolved', async () => {
  const { start, batch, send, ask, hold, retire, shown, locked } = desktop();
  const guard = start();
  await flush();
  ask(1, 'install');
  hold(1, 'install');
  send();
  guard.recheck();
  await flush();
  send();
  assert.deepEqual([shown().map(({ message }) => message), locked()], [[UNRECHECKED], 'update'], 'the batch the request named still holds the install, so Try again stays');
  retire(1);
  batch();
  assert.deepEqual([shown().map(({ message }) => message), locked()], [[UNRECHECKED], 'update'], 'setup failed and its release batch was lost, and the page can still ask');
  shown()[0]!.retry();
  await flush();
  send();
  assert.deepEqual([locked(), shown()], [null, []], 'the batch Try again brought unlocks the page');
});

test('a restart whose reply is lost before its install question is posed keeps Try again through the question and a lost release', async () => {
  const { host, start, batch, send, ask, hold, retire, shown, locked } = desktop();
  const guard = start();
  await flush();
  host.installing = true;
  guard.recheck();
  await flush();
  send();
  assert.deepEqual([locked(), shown()], [null, []], 'the updater is still checking the recorder, so nothing locks and nothing is shown');
  ask(1, 'install');
  hold(1, 'install');
  send();
  assert.deepEqual([locked(), shown().map(({ message }) => message)], ['update', [UNRECHECKED]], 'the install the page asked for locks it, and the lock keeps Try again');
  retire(1);
  batch();
  host.installing = false;
  batch();
  await flush();
  assert.deepEqual([locked(), shown().map(({ message }) => message)], ['update', [UNRECHECKED]], 'the launch failed and both its release batches were lost, and the page can still ask');
  shown()[0]!.retry();
  await flush();
  send();
  assert.deepEqual([locked(), shown()], [null, []], 'the batch Try again brought unlocks the page');
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

type Traced = [string, ...unknown[]];
type Out = [[number, boolean, number[]][], 0 | 1 | 2, number, string[] | null, number];

async function replay(seed: number, trace: Traced[]) {
  const facts: { dirty: boolean | null; confirmed: boolean } = { dirty: false, confirmed: false };
  interface Entry {
    guard?: CloseGuard; onBatch?: (batch: CloseBatch) => boolean; watches: { resolve(): void; reject(cause: unknown): void }[];
    answers: { id: number; resolve(heard: boolean): void; reject(cause: unknown): void }[]; failures: readonly CloseGuardFailure[]; locked: LeavingFor | null; prompts: number;
    refreshes: number; refreshing: Map<number, { resolve(seq: number): void; reject(cause: unknown): void }>;
  }
  const guards: Entry[] = [];
  let emitted: [number, boolean, number[]][] = [];
  const asks = (listed: [number, CloseAbout][]) => listed.map(([id, about]) => ({ id, about }));
  for (const [at, event] of trace.entries()) {
    const where = `seed ${seed}, event ${at}: ${JSON.stringify(event)}`;
    const [kind, index] = event as [string, number];
    emitted = [];
    if (kind === 'm') {
      const entry: Entry = { watches: [], answers: [], failures: [], locked: null, prompts: 0, refreshes: 0, refreshing: new Map() };
      guards[index] = entry;
      entry.guard = closeGuard({
        requests: onBatch => {
          entry.onBatch = onBatch;
          return {
            watch: () => new Promise<void>((resolve, reject) => { entry.watches.push({ resolve, reject }); }),
            refresh: () => { const ordinal = entry.refreshes++; return new Promise<number>((resolve, reject) => { entry.refreshing.set(ordinal, { resolve, reject }); }); },
          };
        },
        answer: (id, stays, basis) => {
          emitted.push([id, stays, [...basis].sort((a, b) => a - b)]);
          return new Promise<boolean>((resolve, reject) => { entry.answers.push({ id, resolve, reject }); });
        },
        leave: () => Promise.resolve(),
      }, { unsaved: () => { if (facts.dirty === null) throw new Error('the drafts couldn’t be read'); return facts.dirty; }, confirmed: () => facts.confirmed, prompt: () => { entry.prompts++; }, leaving: leaving => { entry.locked = leaving; } },
      failures => { entry.failures = failures; });
      continue;
    }
    const entry = guards[index]!;
    if (kind === 'e') { entry.guard!.end(); continue; }
    const prompts = entry.prompts;
    const refreshes = entry.refreshes;
    let out: Out;
    if (kind === 'w') {
      const [, , watched, expected] = event as [string, number, boolean, Out];
      const pending = entry.watches.shift();
      assert.ok(pending, `${where}: the guard asked to watch`);
      if (watched) pending.resolve(); else pending.reject(lost);
      await flush();
      out = expected;
    } else if (kind === 'b') {
      const [, , dirty, confirmed, seq, waiting, holds, returned, expected, installing] = event as [string, number, boolean | null, boolean, number, [number, CloseAbout][], [number, CloseAbout][], boolean, Out, boolean];
      Object.assign(facts, { dirty, confirmed });
      assert.equal(entry.onBatch!({ seq, waiting: asks(waiting), holds: asks(holds), installing }), returned, `${where}: what the callback returned`);
      out = expected;
    } else if (kind === 'r') {
      const [, , dirty, confirmed, id, reply, expected] = event as [string, number, boolean | null, boolean, number, boolean | null, Out];
      Object.assign(facts, { dirty, confirmed });
      const pending = entry.answers.shift();
      assert.equal(pending?.id, id, `${where}: the answer replied to`);
      if (reply === null) pending!.reject(lost); else pending!.resolve(reply);
      await flush();
      out = expected;
    } else if (kind === 'f') {
      const [, , ordinal, seq, expected] = event as [string, number, number, number | null, Out];
      const pending = entry.refreshing.get(ordinal);
      assert.ok(pending, `${where}: the guard asked again what stands`);
      entry.refreshing.delete(ordinal);
      if (seq === null) pending.reject(lost); else pending.resolve(seq);
      await flush();
      out = expected;
    } else if (kind === 'k') {
      entry.guard!.recheck();
      out =(event as [string, number, Out])[2];
    } else if (kind === 't') {
      const [, , dirty, confirmed, key, expected] = event as [string, number, boolean | null, boolean, string, Out];
      Object.assign(facts, { dirty, confirmed });
      const failure = entry.failures.find(shown => shown.key === key);
      assert.ok(failure, `${where}: the failure retried is shown`);
      failure.retry();
      out = expected;
    } else {
      assert.equal(kind, 'l', where);
      entry.guard!.leave();
      out = (event as [string, number, Out])[2];
    }
    const [answers, locked, prompted, failures, refreshed] = out;
    assert.deepEqual(emitted, answers, `${where}: the answers sent`);
    assert.equal(entry.locked, [null, 'close', 'update'][locked], `${where}: whether editing is locked`);
    assert.equal(entry.prompts - prompts, prompted, `${where}: the prompts shown`);
    assert.equal(entry.refreshes - refreshes, refreshed, `${where}: the fresh batches asked for`);
    if (failures !== null) assert.deepEqual(entry.failures.map(({ key }) => key).sort(), failures, `${where}: the failures shown`);
  }
}

test('the real guard answers every randomized interleaving of the host’s test exactly as that test’s model of it did', async () => {
  const traces = JSON.parse(readFileSync(new URL('./closeGuard.traces.json', import.meta.url), 'utf8')) as [number, Traced[]][];
  assert.ok(traces.length > 0);
  for (const [seed, trace] of traces) await replay(seed, trace);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { closeGuard, type CloseGuardFailure, UNCLEARED, UNCLOSED, UNGUARDED, UNHEARD } from './closeGuard.ts';

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
const lost = 'BattlePlan couldn’t reach its desktop host for set_unsaved_work: the IPC request failed (TypeError: Failed to fetch).';

function host(listenFails = false) {
  const marked: boolean[] = [];
  const reports: (CloseGuardFailure | null)[] = [];
  const state = { markFails: false, listenFails, closeFails: false, listens: 0, stopped: 0, closes: 0, prompts: 0 };
  let requested = () => {};
  const guard = closeGuard({
    mark: async unsaved => {
      if (state.markFails) throw lost;
      marked.push(unsaved);
    },
    listen: async onClose => {
      state.listens++;
      if (state.listenFails) throw new Error('no event channel');
      requested = onClose;
      return () => { state.stopped++; };
    },
    close: async () => {
      state.closes++;
      if (state.closeFails) throw 'Couldn’t close BattlePlan: no window';
    },
  }, () => { state.prompts++; }, failure => { reports.push(failure); });
  return { guard, marked, reports, state, close: () => requested() };
}

function pending<T>() {
  const calls: { value: T; settle(applied: boolean, fails?: boolean): void }[] = [];
  const ask = (value: T, apply: (value: T) => void) => new Promise<void>((resolve, reject) => {
    calls.push({ value, settle: (applied, fails = false) => {
      if (applied) apply(value);
      if (fails) reject(lost); else resolve();
    } });
  });
  return { calls, ask };
}

test('unsaved work the host never heard of is said by name and marked again on Try again', async () => {
  const { guard, marked, reports, state } = host();
  await flush();
  assert.deepEqual(reports, [], 'nothing is said while the guard holds');
  state.markFails = true;
  guard.mark(true);
  await flush();
  assert.deepEqual(reports, [{ message: UNGUARDED, detail: lost }]);
  state.markFails = false;
  guard.retry();
  await flush();
  assert.deepEqual(marked, [true], 'Try again marks the work unsaved again');
  assert.deepEqual(reports.at(-1), null, 'and the failure leaves once the host has it');
});

test('saved work the host still thinks unsaved is said too, and a later mark replaces the failed one', async () => {
  const { guard, marked, reports, state } = host();
  state.markFails = true;
  guard.mark(false);
  await flush();
  assert.deepEqual(reports, [{ message: UNCLEARED, detail: lost }]);
  state.markFails = false;
  guard.mark(true);
  await flush();
  assert.deepEqual(marked, [true]);
  assert.equal(reports.at(-1), null);
});

test('a save and a new edit reach the host in order, however the host applies the writes it holds', async () => {
  const marks = pending<boolean>();
  let native: boolean | null = null;
  const apply = (value: boolean) => { native = value; };
  const guard = closeGuard({ mark: value => marks.ask(value, apply), listen: async () => () => {}, close: async () => {} }, () => {}, () => {});
  const settleNewestFirst = async (fails = false) => {
    while (marks.calls.length) {
      for (const call of marks.calls.splice(0).reverse()) call.settle(true, fails);
      await flush();
    }
  };
  guard.mark(true);
  await settleNewestFirst();
  guard.mark(false);
  guard.mark(true);
  assert.equal(marks.calls.length, 1, 'one write is with the host at a time');
  await settleNewestFirst();
  assert.equal(native, true, 'the new edit stays guarded after the save’s late write');

  guard.mark(false);
  guard.mark(true);
  await settleNewestFirst(true);
  assert.equal(native, true, 'a write the host applied but whose reply was lost still comes before the next');
});

test('a close request on clean work closes, and on unsaved work asks, whatever the host last heard', async () => {
  const { guard, reports, state, close } = host();
  await flush();
  state.markFails = true;
  guard.mark(false);
  await flush();
  assert.deepEqual(reports.at(-1), { message: UNCLEARED, detail: lost });
  close();
  await flush();
  assert.deepEqual([state.closes, state.prompts], [1, 0], 'saved work closes at once, with no question kept for later');

  state.markFails = false;
  guard.mark(true);
  await flush();
  close();
  assert.deepEqual([state.closes, state.prompts], [1, 1], 'unsaved work asks');
});

test('a clean close that fails is said by name and Try again asks again from the current work', async () => {
  const { guard, reports, state, close } = host();
  guard.mark(false);
  await flush();
  state.closeFails = true;
  close();
  await flush();
  assert.deepEqual(reports.at(-1), { message: UNCLOSED, detail: 'Couldn’t close BattlePlan: no window' });
  state.closeFails = false;
  guard.mark(true);
  await flush();
  guard.retry();
  await flush();
  assert.deepEqual([state.closes, state.prompts], [1, 1], 'work changed since then is asked about, not closed');
  assert.equal(reports.at(-1), null);
});

test('a close the page cannot hear is said by name and listened for again on Try again', async () => {
  const { guard, reports, state } = host(true);
  await flush();
  assert.deepEqual(reports, [{ message: UNHEARD, detail: 'no event channel' }]);
  state.listenFails = false;
  guard.retry();
  await flush();
  assert.equal(state.listens, 2);
  assert.equal(reports.at(-1), null);
  guard.retry();
  assert.equal(state.listens, 2, 'a guard that holds asks nothing again');
  guard.end();
  assert.equal(state.stopped, 1, 'ending the guard stops listening');
});

test('Try again pressed twice listens once, so no listener is lost and no late failure follows the success', async () => {
  const listens: { resolve(stop: () => void): void; reject(cause: unknown): void }[] = [];
  const reports: (CloseGuardFailure | null)[] = [];
  let stopped = 0;
  const guard = closeGuard({
    mark: async () => {},
    listen: () => new Promise((resolve, reject) => { listens.push({ resolve, reject }); }),
    close: async () => {},
  }, () => {}, failure => { reports.push(failure); });
  listens[0]!.reject(new Error('no event channel'));
  await flush();
  guard.retry();
  guard.retry();
  assert.equal(listens.length, 2, 'a second Try again waits for the first');
  listens[1]!.resolve(() => { stopped++; });
  await flush();
  assert.deepEqual(reports, [{ message: UNHEARD, detail: 'no event channel' }, null]);
  guard.end();
  assert.equal(stopped, 1);
});

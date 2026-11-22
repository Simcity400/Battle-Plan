import assert from 'node:assert/strict';
import test from 'node:test';
import { closeGuard, type CloseGuardFailure, UNCLEARED, UNGUARDED, UNHEARD } from './closeGuard.ts';

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
const lost = 'BattlePlan couldn’t reach its desktop host for set_unsaved_work: the IPC request failed (TypeError: Failed to fetch).';

function host(listenFails = false) {
  const marked: boolean[] = [];
  const reports: (CloseGuardFailure | null)[] = [];
  const state = { markFails: false, listenFails, listens: 0, stopped: 0 };
  const guard = closeGuard({
    mark: async unsaved => {
      if (state.markFails) throw lost;
      marked.push(unsaved);
    },
    listen: async () => {
      state.listens++;
      if (state.listenFails) throw new Error('no event channel');
      return () => { state.stopped++; };
    },
  }, () => {}, failure => { reports.push(failure); });
  return { guard, marked, reports, state };
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

import assert from 'node:assert/strict';
import test from 'node:test';
import { EventEmitter } from 'node:events';
import { setImmediate as nextTurn } from 'node:timers/promises';
import type { ChildProcess } from 'node:child_process';
import { browserStage, stopBrowser, stopOwnedProfile } from './canvas-page.ts';
import type { BrowserCleanupBudget } from './canvas-page.ts';

test('Canvas stages stop on browser exit, spawn failure and a missing page result', async () => {
  const child = Object.assign(new EventEmitter(), { exitCode: null as number | null, signalCode: null }) as unknown as ChildProcess;
  const waiting = () => new Promise<never>(() => {});
  const exited = browserStage(child, 'startup', waiting);
  child.emit('close', 1, null);
  await assert.rejects(exited, /startup: Edge exited/);
  assert.equal(child.listenerCount('close'), 0);
  const failed = browserStage(child, 'startup', waiting);
  child.emit('error', new Error('synthetic spawn failure'));
  await assert.rejects(failed, /synthetic spawn failure/);
  assert.equal(child.listenerCount('error'), 0);
  let signal: AbortSignal | undefined;
  await assert.rejects(browserStage(child, 'page', current => { signal = current; return waiting(); }, 5), /page timed out/);
  assert.equal(signal?.aborted, true, 'endpoint polling can stop when the stage ends');
  assert.equal(await browserStage(child, 'page', async () => 42), 42);
  child.exitCode = 1;
  await assert.rejects(browserStage(child, 'already stopped', async () => assert.fail('must not start work')), /already stopped: Edge exited/);
  assert.equal(child.listenerCount('close'), 0);
  assert.equal(child.listenerCount('error'), 0);
});

function browser() {
  return Object.assign(new EventEmitter(), { exitCode: null as number | null, signalCode: null, unref() {} }) as unknown as ChildProcess;
}

test('Canvas shutdown observes an immediate graceful close before requesting it', async () => {
  const child = browser();
  const otherObserver = () => {};
  child.on('close', otherObserver);
  await stopBrowser(child, async () => {
    assert.equal(child.listenerCount('close'), 2);
    child.exitCode = 0; child.emit('close', 0, null);
  }, async () => assert.fail('a graceful close must not force termination'));
  assert.equal(child.listenerCount('close'), 1);
});

test('Canvas shutdown waits for its owned process tree after a graceful-close deadline', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const child = browser();
  let forced = false;
  const stopped = stopBrowser(child, async () => {}, async () => {
    forced = true;
    // The force request completes before the process/pipe close notification.
    setTimeout(() => { child.signalCode = 'SIGKILL'; child.emit('close', null, 'SIGKILL'); }, 10);
  });
  await nextTurn();
  assert.equal(forced, false);
  context.mock.timers.tick(5_000);
  await nextTurn();
  assert.equal(forced, true);
  context.mock.timers.tick(10);
  await stopped;
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas shutdown retains failure when forced termination never produces an exit', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const child = browser();
  let forced = false;
  const stopped = stopBrowser(child, async () => { throw new Error('unavailable endpoint'); }, async () => { forced = true; });
  const refused = assert.rejects(stopped, /Headless Edge did not stop; its profile is retained/);
  await nextTurn();
  context.mock.timers.tick(5_000);
  await nextTurn();
  assert.equal(forced, true);
  context.mock.timers.tick(5_000);
  await refused;
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas shutdown confirms a graceful exit racing its force request', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const child = browser();
  const stopped = stopBrowser(child, async () => {}, async () => {
    child.exitCode = 0; child.emit('close', 0, null);
    throw new Error('the owned process already exited');
  });
  await nextTurn();
  context.mock.timers.tick(5_000);
  await stopped;
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas shutdown accepts native family absence when Node exit notification is stale', async () => {
  const child = browser();
  let released = false;
  child.unref = () => { released = true; };
  await stopBrowser(child, async () => {}, async () => assert.fail('the verified native family already stopped'), 5_000, async () => true);
  assert.equal(child.exitCode, null, 'native confirmation does not fabricate a Node exit status');
  assert.equal(released, true);
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas shutdown never accepts a vanished launcher while an owned family remains', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const child = browser();
  child.exitCode = 0;
  let familyStopped = false, forced = false;
  const stopped = stopBrowser(child, async () => {}, async () => {
    forced = true; familyStopped = true; child.emit('close', 0, null);
  }, 5_000, async () => familyStopped);
  await nextTurn();
  assert.equal(forced, false);
  context.mock.timers.tick(5_000);
  await stopped;
  assert.equal(forced, true);
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas shutdown cleans the owned family after its process observation times out', async () => {
  const child = browser();
  const profile = 'C:\\Temp\\share-canvas-synthetic';
  const processes = new Map([[11, profile], [12, profile], [21, `${profile}-another`], [22, 'C:\\Temp\\other-profile']]);
  let observations = 0, forced = false;
  await stopBrowser(child, async () => { throw new Error('unavailable endpoint'); }, async () => {
    forced = true;
    // A detached browser outlives the launcher tree. Cleanup must sweep the
    // exact owned profile while retaining other runs and similar prefixes.
    processes.delete(11);
    for (const [pid, owner] of processes) if (owner === profile) processes.delete(pid);
    child.exitCode = 0; child.emit('close', 0, null);
  }, 1, async () => {
    if (++observations === 1) throw new Error('Observing owned Headless Edge processes timed out');
    return ![...processes.values()].includes(profile);
  });
  assert.equal(forced, true, 'an observation failure must still terminate the owned browser');
  assert.deepEqual([...processes.keys()], [21, 22], 'no owned Edge survives; unrelated profiles stay alive');
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas profile cleanup retries observer timeouts and sweeps relaunched survivors until none remain', async () => {
  const profile = 'C:\\Temp\\share-canvas-synthetic';
  const processes = new Map([[11, profile], [12, profile], [21, `${profile}-another`], [22, 'C:\\Temp\\other-profile']]);
  let stops = 0, verifications = 0;
  await stopOwnedProfile(profile, 11, 12, async (owner, launcherPid, stop, browserPid) => {
    assert.equal(owner, profile, 'every sweep is scoped to the exact run profile');
    if (stop) {
      assert.equal(launcherPid, stops < 2 ? 11 : undefined, 'retain the launcher check through a timed-out stop, then rely on exact-profile ownership');
      assert.equal(browserPid, 12);
      if (++stops === 1) throw new Error('Observing owned Headless Edge processes timed out');
      if (stops === 2) processes.delete(11); // The tree kill misses a detached browser.
      if (stops === 3) { processes.delete(12); processes.set(13, profile); } // A relaunch races the first sweep.
      if (stops === 4) processes.delete(13);
    } else if (++verifications === 1) throw new Error('Observing owned Headless Edge processes timed out');
    return [...processes].filter(([, path]) => path === owner).map(([pid]) => ({ pid, parentId: 0 }));
  });
  assert.equal(stops, 4, 'a timeout or surviving root cannot end cleanup');
  assert.equal(verifications, 3);
  assert.deepEqual([...processes.keys()], [21, 22], 'every owned browser stopped without terminating other profiles');
});

test('Canvas shutdown has one deadline through permanently timed-out observations and its final owned-handle stop', async context => {
  context.mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  const child = browser();
  const profile = 'C:\\Temp\\share-canvas-permanent-timeout';
  let nativeStops = 0, finalSweeps = 0, calls = 0, settled = false, failure: unknown;
  child.pid = 11;
  child.kill = signal => { assert.equal(signal, 'SIGKILL'); nativeStops++; return true; };
  const observe = async (owner: string, launcher: number | undefined, stop = false, _browserPid?: number, signal?: AbortSignal) => {
    assert.equal(owner, profile);
    if (stop) assert.equal(launcher, 11, 'a timed-out sweep retains its ownership check');
    calls++;
    return new Promise<never>((_, reject) => {
      const timer = setTimeout(() => reject(new Error('Observing owned Headless Edge processes timed out')), 10);
      signal?.addEventListener('abort', () => { clearTimeout(timer); reject(signal.reason); }, { once: true });
    });
  };
  const stopped = stopBrowser(child, async () => {}, async budget => {
    if (budget?.final) finalSweeps++;
    await stopOwnedProfile(profile, 11, 12, observe, budget);
  }, 5, async (_budget, signal) => (await observe(profile, undefined, false, undefined, signal)).length === 0,
  { profile, timeoutMs: 40, finalAttemptMs: 10 });
  void stopped.then(() => { settled = true; }, error => { settled = true; failure = error; });
  await nextTurn();
  for (let elapsed = 0; elapsed < 40; elapsed += 5) { context.mock.timers.tick(5); await nextTurn(); }
  assert.equal(settled, true, 'permanent CIM timeouts cannot extend the shared cleanup deadline');
  assert.ok(failure instanceof Error);
  assert.ok(failure.message.includes(profile), 'failure names the profile retained for inspection');
  assert.equal(nativeStops, 1, 'a final stop uses the retained native child handle when observation remains unavailable');
  assert.equal(finalSweeps, 1, 'one final exact-profile attempt shares the remaining deadline');
  assert.ok(calls <= 5, 'nested cleanup cannot restart the retry deadline');
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas shutdown bounds hanging close, force and verification callbacks', async context => {
  context.mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  const child = browser();
  const profile = 'C:\\Temp\\share-canvas-hanging-callbacks';
  const signals: AbortSignal[] = [];
  let finalStops = 0, settled = false, failure: unknown;
  const hanging = async (_budget: BrowserCleanupBudget, signal: AbortSignal) => {
    signals.push(signal);
    return new Promise<never>(() => {});
  };
  const stopped = stopBrowser(child, hanging, async (budget, signal) => {
    if (budget?.final) finalStops++;
    await hanging(budget, signal);
  }, 5, hanging, { profile, timeoutMs: 40, finalAttemptMs: 10 });
  void stopped.then(() => { settled = true; }, error => { settled = true; failure = error; });
  await nextTurn();
  for (let elapsed = 0; elapsed < 40; elapsed += 5) { context.mock.timers.tick(5); await nextTurn(); }
  assert.equal(settled, true, 'even callbacks that ignore cancellation settle within the overall deadline');
  assert.ok(failure instanceof Error && failure.message.includes(profile));
  assert.equal(finalStops, 1, 'hanging verification still leaves a bounded final stop attempt');
  assert.equal(signals.length, 3);
  assert.ok(signals.every(signal => signal.aborted), 'started resources receive cancellation when their step ends');
  assert.equal(child.listenerCount('close'), 0);
});

test('Canvas standalone profile cleanup has a finite shared attempt allowance', async () => {
  const profile = 'C:\\Temp\\share-canvas-retry-limit';
  let observations = 0;
  await assert.rejects(stopOwnedProfile(profile, 11, 12, async () => {
    if (++observations > 100) throw new Error('Synthetic safeguard after unbounded retries');
    throw new Error('Observing owned Headless Edge processes timed out');
  }), error => error instanceof Error && error.message.includes(profile));
  assert.equal(observations, 21, 'immediate timeout rejections cannot starve the event loop forever');
});

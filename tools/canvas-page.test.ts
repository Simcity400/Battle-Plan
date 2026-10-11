import assert from 'node:assert/strict';
import test from 'node:test';
import { EventEmitter } from 'node:events';
import { setImmediate as nextTurn } from 'node:timers/promises';
import type { ChildProcess } from 'node:child_process';
import { browserStage, stopBrowser } from './canvas-page.ts';

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

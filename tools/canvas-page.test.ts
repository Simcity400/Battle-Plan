import assert from 'node:assert/strict';
import test from 'node:test';
import { EventEmitter } from 'node:events';
import type { ChildProcess } from 'node:child_process';
import { browserStage } from './canvas-page.ts';

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

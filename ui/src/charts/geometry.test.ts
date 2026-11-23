import assert from 'node:assert/strict';
import test from 'node:test';
import { FrameEngine } from '../engine/frame.ts';
import { GeometryReads } from './geometry.ts';

function frames() {
  let asked: ((now: number) => void) | null = null;
  const engine = new FrameEngine({ request: run => { asked = run; } });
  const log: string[] = [], soon: (() => void)[] = [];
  const reads = new GeometryReads(engine, writes => { log.push('commit writes'); writes(); }, run => { soon.push(run); });
  const read = (name: string) => () => { log.push('read ' + name); return () => { log.push('write ' + name); }; };
  return { engine, reads, read, log, soon, run() { const run = asked; asked = null; assert.ok(run, 'a frame was asked for'); run(16); } };
}

test('geometry asked for by many commits in one frame is read once, after the frame commits, every read before any write', () => {
  const page = frames(), a = page.read('a'), b = page.read('b'), dropped = page.read('dropped');
  page.engine.next('commit', () => { page.log.push('commit 1'); page.reads.invalidate(a); page.reads.invalidate(dropped); });
  page.engine.next('commit', () => { page.log.push('commit 2'); page.reads.invalidate(b); page.reads.invalidate(a); page.reads.cancel(dropped); });
  page.run();
  assert.deepEqual(page.log, ['commit 1', 'commit 2', 'read a', 'read b', 'commit writes', 'write a', 'write b']);
  assert.deepEqual(page.soon, [], 'nothing waits for a microtask');
});

test('geometry asked for between frames is read right after that commit; asked for by a frame’s writes, in the next frame', () => {
  const page = frames(), later = page.read('later');
  const first = () => { page.log.push('read first'); return () => { page.log.push('write first'); }; };
  page.reads.invalidate(first); page.reads.invalidate(first);
  assert.equal(page.soon.length, 1, 'one read is asked for, without waiting for a frame');
  page.soon.shift()!();
  assert.deepEqual(page.log, ['read first', 'commit writes', 'write first']);
  const writes = () => { page.log.push('read writes'); return () => { page.reads.invalidate(later); }; };
  page.engine.next('commit', () => page.reads.invalidate(writes));
  page.run();
  assert.deepEqual(page.log.slice(3), ['read writes', 'commit writes']);
  page.run();
  assert.deepEqual(page.log.slice(5), ['read later', 'commit writes', 'write later']);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { inStructureSlot, StructurePace, STRUCTURE_SLOT_MS, structureOpen } from './structurePace.ts';

// The engine's frames, driven by hand: a frame is run at a time past the next slot's start, never waited for.
const frames: ((now: number) => void)[] = [];
globalThis.requestAnimationFrame = (run => frames.push(run)) as typeof requestAnimationFrame;
const slot = () => { const now = performance.now() + 2 * STRUCTURE_SLOT_MS; for (const run of frames.splice(0)) run(now); };

test('while playing, a structure is taken only in the slot\'s task, every one held taken there together with its newest value; paused, at once', () => {
  const applied: string[] = [];
  const rows = new StructurePace<string>('a', Object.is, value => applied.push(`rows ${value}`));
  const order = new StructurePace<string>('1', Object.is, value => applied.push(`order ${value}`));
  rows.offer('b', true);
  order.offer('2', true);
  rows.offer('c', true);
  rows.offer('d', true);
  assert.deepEqual(applied, [], 'playing, nothing is taken outside the slot, not even the first change');
  slot();
  assert.deepEqual(applied.sort(), ['order 2', 'rows d'], 'at the slot both are taken, the newest rows only');
  inStructureSlot(() => rows.offer('e', true));
  assert.equal(applied.at(-1), 'rows e', 'offered inside the slot\'s task, taken there');
  order.offer('3', false);
  assert.equal(applied.at(-1), 'order 3', 'paused, a change is taken at once');
  rows.offer('f', true);
  rows.offer('e', true);
  slot();
  assert.equal(applied.filter(entry => entry === 'rows f').length, 0, 'a change offered back to the value held is taken nowhere');
});

test('a structure may be taken paused, or playing only inside the slot\'s task', () => {
  assert.equal(structureOpen(false), true);
  assert.equal(structureOpen(true), false);
  let inside = false;
  inStructureSlot(() => { inside = structureOpen(true); });
  assert.equal(inside, true);
  assert.equal(structureOpen(true), false, 'closed again after the task');
});

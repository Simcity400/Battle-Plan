import assert from 'node:assert/strict';
import test from 'node:test';
import { registerDraft, settled, unsavedNow, writeSlot, writesState } from './pendingWrites.ts';

test('a write is pending from the moment it is asked until it is answered, and a failed one until it saves or is discarded', async () => {
  assert.deepEqual([unsavedNow(), writesState()], [false, null]);
  const slot = writeSlot();
  slot.enter();
  slot.enter();
  assert.deepEqual([unsavedNow(), writesState()], [true, 'saving'], 'a second edit while the first is on its way stays one pending write');
  slot.failed();
  assert.deepEqual([unsavedNow(), writesState()], [true, 'failed'], 'a failure keeps its edit unsaved for the close answer');
  assert.equal(await settled(), false, 'Save… doesn’t resume the close over a change that failed');
  slot.enter();
  const resumed = settled();
  slot.saved();
  assert.equal(await resumed, true, 'once the retry saves, the requested close can go ahead');
  assert.deepEqual([unsavedNow(), writesState()], [false, null]);
  slot.enter();
  slot.failed();
  slot.discard();
  assert.equal(unsavedNow(), false, 'a discarded failure leaves nothing pending');
});

test('drafts count as unsaved only while their reader says so', () => {
  let editing = true;
  const unregister = registerDraft(() => editing);
  assert.equal(unsavedNow(), true);
  editing = false;
  assert.equal(unsavedNow(), false);
  editing = true;
  unregister();
  assert.equal(unsavedNow(), false);
});

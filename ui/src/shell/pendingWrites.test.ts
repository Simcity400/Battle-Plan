import assert from 'node:assert/strict';
import test from 'node:test';
import { draftSection, registerDraft, retryFailed, settingDraft, settled, unsavedNow, watchWrites, writeSlot, writesState } from './pendingWrites.ts';
import { tracked } from './writes.ts';

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
  let retried = 0;
  slot.enter();
  slot.failed(() => { retried++; slot.enter(); });
  retryFailed();
  assert.deepEqual([retried, writesState()], [1, 'saving'], 'Save… sends each failed change again through its own retry');
  slot.saved();
});

test('every write the client sends is pending from the moment it is sent until it is answered, whatever the answer', async () => {
  let answer!: (value: string) => void;
  const sent = tracked(() => new Promise<string>(resolve => { answer = resolve; }));
  assert.equal(writesState(), 'saving');
  answer('done');
  assert.equal(await sent, 'done');
  assert.equal(writesState(), null);
  await assert.rejects(tracked(() => Promise.reject(new Error('refused'))));
  assert.equal(writesState(), null, 'a failed write the form keeps no draft of leaves nothing behind');
});

test('a settings field typed but not applied keeps a close from going ahead unasked', () => {
  let typed = true;
  const unregister = registerDraft(() => typed, 'folders');
  assert.equal(writesState(), 'failed');
  assert.equal(draftSection(), 'folders', 'Save… can open the settings that hold it');
  typed = false;
  assert.equal(writesState(), null);
  unregister();
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

test('a settings draft is kept by the registry, not its field, and tells the shell when it turns dirty or clean', () => {
  const typed = settingDraft<string>('folders');
  let heard = 0;
  const stop = watchWrites(() => { heard++; });
  typed.set('D:/Recordings');
  assert.deepEqual([typed.value(), writesState(), draftSection(), heard], ['D:/Recordings', 'failed', 'folders', 1], 'a field opened again finds the draft the user left');
  typed.set('D:/Recordings/2026');
  assert.equal(heard, 1, 'editing a draft that stays dirty tells nobody');
  typed.set(null);
  assert.deepEqual([writesState(), heard], [null, 2], 'typing the saved path back clears it and wakes a close waiting on it');
  stop();
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { datedPanel } from './messages.fixture.ts';
import { restorePanelBatches } from './panelWire.ts';

const buffer = (bytes: Uint8Array) => bytes.slice().buffer;
const mark = (serial: number) => new Uint32Array([0x31425750, serial]).buffer;
const fingerprint = (bytes: Uint8Array) => bytes.reduce(
  (hash, value) => Math.imul(hash ^ value, 0x01000193) >>> 0, 0x811c9dc5,
);

function packet(id: number, seek: number) {
  const base = datedPanel(id, 20, 'x'.repeat(250) + '1', seek);
  const whole = datedPanel(id, 21, 'x'.repeat(250) + '9', seek);
  const length = new DataView(whole.buffer).getUint32(36, true);
  const oldBody = base.subarray(40, 40 + length), newBody = whole.subarray(40, 40 + length);
  const patched = new Uint8Array(72);
  patched.set(whole.subarray(0, 40));
  const data = new DataView(patched.buffer);
  data.setUint32(0, 0x31444e50, true);
  data.setUint8(32, 8);
  data.setUint32(36, 25, true);
  data.setUint32(40, fingerprint(oldBody), true);
  data.setUint32(44, length, true);
  data.setUint32(48, fingerprint(newBody), true);
  data.setUint32(52, length - 1, true);
  data.setUint32(56, 0, true);
  data.setUint32(60, 0x8000_0001, true);
  patched[64] = newBody[length - 1]!;
  return { base: buffer(base), whole: buffer(whole), patched: buffer(patched) };
}

test('the pull reader delivers the exact dated packet after restoring the changed bytes', async () => {
  const offered: ArrayBuffer[] = [];
  const restore = restorePanelBatches(bytes => offered.push(bytes), async () => assert.fail('the base is present'));
  const { base, whole, patched } = packet(1, 0);
  await restore([base]);
  await restore([mark(1), patched]);
  assert.deepEqual(offered, [base, whole]);
});

test('a delta for a different seek waits for the original batch before delivering any of its packets', async () => {
  const offered: ArrayBuffer[] = [];
  const { base } = packet(1, 0);
  const { whole, patched } = packet(1, 1);
  const earlier = buffer(datedPanel(2, 21, { hour: 21 }, 1));
  const restore = restorePanelBatches(bytes => offered.push(bytes), async serial => {
    assert.equal(serial, 13);
    assert.deepEqual(offered, [base], 'no partial batch was sent');
    return [earlier, whole];
  });
  await restore([base]);
  await restore([mark(13), earlier, patched]);
  assert.deepEqual(offered, [base, earlier, whole]);
});

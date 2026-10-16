import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bridgeCommand, mapSettingsCommand, recordingRequestBody } from '../../browserBridge.ts';
import { MAX_MESSAGE_BYTES } from '../generated/dashboard.ts';

async function* body(...chunks: Buffer[]) { yield* chunks; }

function product(tags: string[]) {
  return { type: 'product', id: 'request-1', request: { sourceId: 'source-1', revision: '1', product: 'agency-comparison', params: { branchId: '1', tags, top: null } } };
}

test('browser HTTP and native bridge requests keep all 5,000 synthetic countries', async () => {
  const tags = Array.from({ length: 5_000 }, (_, index) => `synthetic-country-${index.toString().padStart(4, '0')}`);
  const command = product(tags);
  const wire = bridgeCommand(command);
  assert.ok(Buffer.byteLength(wire) > 64 * 1024);
  assert.deepEqual(JSON.parse(wire), command);
  assert.deepEqual(await recordingRequestBody(body(Buffer.from(wire))), command);
});

test('browser HTTP requests accept the full byte budget and refuse one byte past it', async () => {
  const wire = JSON.stringify(product(['']));
  const exact = Buffer.from(JSON.stringify(product(['x'.repeat(MAX_MESSAGE_BYTES - Buffer.byteLength(wire))])));
  assert.equal(exact.length, MAX_MESSAGE_BYTES);
  assert.deepEqual(await recordingRequestBody(body(exact)), JSON.parse(exact.toString()));
  await assert.rejects(recordingRequestBody(body(exact, Buffer.from(' '))), { message: 'Recording request too large' });
});

test('browser bridge commands include the native envelope and newline in their byte budget', () => {
  const overhead = Buffer.byteLength(bridgeCommand(product([''])));
  const command = product(['x'.repeat(MAX_MESSAGE_BYTES - overhead)]);
  assert.equal(Buffer.byteLength(bridgeCommand(command)), MAX_MESSAGE_BYTES);
  command.request.params.tags[0] += 'x';
  assert.throws(() => bridgeCommand(command), { message: 'Bridge command exceeds its byte budget' });
});

test('a browser map-settings flip reaches the native bridge with its setting', () => {
  // The body browserClient.flipMapSetting posts, through the bridge's command and its line framing.
  const flip = { type: 'mapSettings', sourceId: 's', revision: 'r', settings: null, flip: 'dayNight', target: { scope: 'widget', id: 'w' } };
  const line = JSON.parse(bridgeCommand({ ...mapSettingsCommand(flip), id: 'request-1' }));
  assert.deepEqual(line, {
    type: 'mapSettings', id: 'request-1', sourceId: 's', revision: 'r', settings: null, flip: 'dayNight', target: { scope: 'widget', id: 'w' },
  });
  // A save carries its settings and no flip; an older body without either reads.
  const save = mapSettingsCommand({ type: 'mapSettings', sourceId: 's', revision: 'r', settings: { graphics: {} }, flip: null });
  assert.deepEqual([save.settings, save.flip, save.target], [{ graphics: {} }, null, { scope: 'campaign' }]);
  const read = mapSettingsCommand({ type: 'mapSettings', sourceId: 's', revision: 'r' });
  assert.deepEqual([read.settings, read.flip], [null, null]);
});

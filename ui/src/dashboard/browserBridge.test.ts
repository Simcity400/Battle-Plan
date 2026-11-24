import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bridgeCommand, pageRecordings, preparedQueryCommand, recordingRequestBody } from '../../browserBridge.ts';
import { MAX_MESSAGE_BYTES } from '../generated/dashboard.ts';
import { productRequest } from './request.ts';

async function* body(...chunks: Buffer[]) { yield* chunks; }

function product(tags: string[]) {
  return { type: 'product', id: 'request-1', request: productRequest({ sourceId: 'source-1', revision: '1', product: 'agency-comparison', params: { branchId: '1', tags, top: null } }) };
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
  (command.request.params.tags as string[])[0] += 'x';
  assert.throws(() => bridgeCommand(command), { message: 'Bridge command exceeds its byte budget' });
});


test('a browser prepared-query refresh reaches the native bridge as a refresh', () => {
  const request = productRequest({ sourceId: 's', revision: 'r', product: 'p', params: {} });
  assert.equal(preparedQueryCommand({ type: 'preparedQuery', request, refresh: true }).refresh, true);
  assert.equal(preparedQueryCommand({ type: 'preparedQuery', request, refresh: false }).refresh, false);
  assert.equal(preparedQueryCommand({ type: 'preparedQuery', request }).refresh, false, 'a body without the flag is a first ask');
  assert.throws(() => preparedQueryCommand({ type: 'preparedQuery', request, refresh: 'yes' }), { message: 'Invalid prepared query request' });
});

test('a page that has gone has every source it holds released, and its imports go once nothing holds them', () => {
  const released: string[] = [], removed: string[] = [];
  const pages = pageRecordings(sourceId => { released.push(sourceId); }, target => { removed.push(target); });
  pages.imported('first.battleplan', 'page-1');
  pages.open('first.battleplan', 'page-1')('lease-1');
  pages.open('library.battleplan', 'page-1')('lease-2');
  pages.released('lease-1');
  assert.deepEqual([released, removed], [[], []], 'the page that imported it can still open it again');
  pages.open('first.battleplan', 'page-1')('lease-3');
  const late = pages.open('library.battleplan', 'page-1');
  pages.left('page-1');
  assert.deepEqual(released, ['lease-2', 'lease-3'], 'every source the page still holds is released, known to it or not');
  assert.deepEqual(removed, ['first.battleplan']);
  assert.equal(pages.isImport('first.battleplan'), false, 'a removed import cannot be opened');
  late('lease-4'); late('lease-4');
  assert.deepEqual(released, ['lease-2', 'lease-3', 'lease-4'], 'an open its page left during is released as it ends, once');
  pages.released('lease-2'); pages.released('unknown-lease'); pages.left('page-1');
  assert.equal(released.length, 3, 'nothing is released twice');

  pages.imported('second.battleplan', 'page-2');
  const checking = pages.open('second.battleplan', 'page-2');
  pages.left('page-2');
  assert.equal(pages.isImport('second.battleplan'), true);
  checking();
  assert.deepEqual(removed, ['first.battleplan', 'second.battleplan'], 'a refused open holds nothing once it ends');
  pages.imported('third.battleplan', 'page-3');
  pages.open('third.battleplan', 'page-4')('lease-5');
  pages.left('page-3');
  assert.equal(removed.length, 2);
  pages.released('lease-5');
  assert.equal(removed.at(-1), 'third.battleplan');
  pages.imported('fourth.battleplan', 'page-2');
  assert.equal(removed.at(-1), 'fourth.battleplan', 'an import finished after its page left is removed at once');
});

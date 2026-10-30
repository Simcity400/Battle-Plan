import assert from 'node:assert/strict';
import test from 'node:test';
import { loadProduct } from './products.ts';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';

const settle = () => new Promise(resolve => setTimeout(resolve, 0));

/** A client whose prepared reads wait until answered, each kept with its signal. */
function transport() {
  const asked: { signal: AbortSignal; answer: (value: unknown) => void }[] = [];
  const client = {
    preparedQuery: (_source: unknown, _product: string, _params: unknown, options: { signal: AbortSignal }) =>
      new Promise(resolve => asked.push({ signal: options.signal, answer: resolve })),
  } as unknown as DashboardClient;
  return { client, asked };
}
const source = { id: 'shared-loads', revision: '1' } as DashboardSource;
const request = (window: number): ProductRequest => ({ sourceId: source.id, revision: source.revision, product: 'land-battles', params: { branchId: '1', window } });
const final = (value: number) => ({ value, coverage: { firstHour: 0, lastHour: 10, complete: true }, pending: [] });

test('mounts asking the same prepared product share one read, and each is told its reply', async () => {
  const { client, asked } = transport();
  const told: [string, unknown][] = [];
  const mount = (name: string) => loadProduct(client, request(1), new AbortController().signal, undefined, source, false,
    value => told.push([name, value]));
  const first = mount('first'), second = mount('second');
  await settle();
  assert.equal(asked.length, 1, 'one read for both');
  asked[0].answer(final(1));
  assert.deepEqual(await Promise.all([first, second]), [final(1), final(1)]);
  assert.deepEqual(told.map(([name]) => name).sort(), ['first', 'second']);
  // Its final reply is kept, so a later mount reads nothing.
  await mount('later');
  assert.equal(asked.length, 1);
});

test('a mount leaving keeps the shared read for the rest, and the last one leaving cancels it', async () => {
  const { client, asked } = transport();
  const leaving = new AbortController(), staying = new AbortController();
  const left = loadProduct(client, request(2), leaving.signal, undefined, source, false, () => {});
  const kept = loadProduct(client, request(2), staying.signal, undefined, source, false, () => {});
  await settle();
  leaving.abort();
  await assert.rejects(left, /cancelled/);
  await settle();
  assert.equal(asked[0].signal.aborted, false, 'still read for the mount that stays');
  asked[0].answer(final(2));
  assert.deepEqual(await kept, final(2));
  const alone = new AbortController();
  const last = loadProduct(client, request(3), alone.signal, undefined, source, false, () => {});
  await settle();
  alone.abort();
  await assert.rejects(last, /cancelled/);
  await settle();
  assert.equal(asked[1].signal.aborted, true, 'nobody left to read for');
});

test('a retry reads afresh rather than joining the read in flight', async () => {
  const { client, asked } = transport();
  const reading = loadProduct(client, request(4), new AbortController().signal, undefined, source, false, () => {});
  const retried = loadProduct(client, request(4), new AbortController().signal, undefined, source, true, () => {});
  await settle();
  assert.equal(asked.length, 2);
  for (const ask of asked) ask.answer(final(4));
  assert.deepEqual(await Promise.all([reading, retried]), [final(4), final(4)]);
});

test('a mount whose own validation refuses a reply fails alone, and the refused reply is never kept', async () => {
  const { client, asked } = transport();
  // Refuses only its first look, so a second look would cache what it refused.
  let looks = 0;
  const refuse = (data: unknown) => { if (++looks === 1) throw new Error('refused'); return data; };
  const refusing = loadProduct(client, request(5), new AbortController().signal, refuse, source, false, () => {});
  const accepting = loadProduct(client, request(5), new AbortController().signal, data => data, source, false, () => {});
  await settle();
  asked[0].answer(final(5));
  await assert.rejects(refusing, /refused/);
  assert.deepEqual(await accepting, final(5));
  assert.equal(looks, 1, 'the refused reply is validated once');
  const later = loadProduct(client, request(5), new AbortController().signal, undefined, source, false, () => {});
  await settle();
  assert.equal(asked.length, 2, 'nothing refused was cached, so a later mount reads again');
  asked[1].answer(final(5));
  assert.deepEqual(await later, final(5));
});

test('a mount failing as it joins leaves the read to the others, and the last one leaving still cancels it', async () => {
  const { client, asked } = transport();
  const statuses: AbortSignal[] = [];
  Object.assign(client, { preparationStatus: (_request: unknown, signal: AbortSignal) => new Promise(() => { statuses.push(signal); }) });
  const staying = new AbortController();
  const kept = loadProduct(client, request(6), staying.signal, undefined, source, false, () => {});
  await settle();
  asked[0].answer({ value: 6, coverage: { firstHour: 0, lastHour: 5, complete: false }, pending: [] });
  await settle();
  assert.equal(statuses.length, 1, 'waiting for preparation to cover the rest');
  const late = loadProduct(client, request(6), new AbortController().signal, () => { throw new Error('refused'); }, source, false, () => {});
  await assert.rejects(late, /refused/);
  staying.abort();
  await assert.rejects(kept, /cancelled/);
  await settle();
  assert.equal(statuses[0].aborted, true, 'nobody left to read for');
});

test('replies that follow the hour never push out those that do not, and either answers a later ask', async () => {
  const asked: string[] = [];
  const client = { product: async (request: ProductRequest) => { asked.push(JSON.stringify(request.params)); return { ...request, data: { at: request.params } }; } } as unknown as DashboardClient;
  const plain = { id: 'hour-cache', revision: '1' } as DashboardSource;
  const at = (params: unknown): ProductRequest => ({ sourceId: plain.id, revision: plain.revision, product: 'synthetic-catalogue', params });
  const signal = () => new AbortController().signal;
  await loadProduct(client, at({ whole: true }), signal());
  // Far more hour replies than the shared cache holds.
  for (let hour = 0; hour < 300; hour++) await loadProduct(client, at({ hour }), signal(), undefined, undefined, false, undefined, undefined, true);
  await loadProduct(client, at({ whole: true }), signal());
  assert.equal(asked.filter(params => params === JSON.stringify({ whole: true })).length, 1, 'the reply that does not follow the hour is still held');
  await loadProduct(client, at({ hour: 299 }), signal());
  assert.equal(asked.filter(params => params === JSON.stringify({ hour: 299 })).length, 1, 'an hour reply answers an ordinary ask too');
});

test('a held reply is validated each consumer’s own way, and an ask cancelled at once is cancelled even when the reply is held', async () => {
  const client = { product: async (request: ProductRequest) => ({ ...request, data: { rows: ['held'] } }) } as unknown as DashboardClient;
  const plain = { id: 'own-validation', revision: '1' } as DashboardSource;
  const request: ProductRequest = { sourceId: plain.id, revision: plain.revision, product: 'synthetic-rows', params: {} };
  const signal = () => new AbortController().signal;
  const rows = (data: unknown) => (data as { rows: string[] }).rows;
  // Settled by a read that follows the hour, validated its way.
  assert.deepEqual(await loadProduct(client, request, signal(), rows, undefined, false, undefined, undefined, true), ['held']);
  assert.deepEqual(await loadProduct(client, request, signal(), (data: unknown) => data), { rows: ['held'] }, 'another consumer is given its own validation');
  const cancelled = new AbortController();
  const asked = loadProduct(client, request, cancelled.signal);
  cancelled.abort();
  await assert.rejects(asked, /cancelled/);
});

test('a held answer cancelled after it settled but before it is told is cancelled, and tells nothing', async () => {
  const { client, asked } = transport();
  const held = request(7);
  const first = loadProduct(client, held, new AbortController().signal, undefined, source, false, () => {});
  await settle();
  asked[0].answer(final(7));
  await first;
  const told: unknown[] = [];
  const cancelled = new AbortController();
  const late = loadProduct(client, held, cancelled.signal, undefined, source, false, value => told.push(value));
  queueMicrotask(() => cancelled.abort());
  await assert.rejects(late, /cancelled/);
  assert.deepEqual(told, []);
});

test('an ask joins the same load going in the other cache rather than asking again', async () => {
  const answers: ((data: unknown) => void)[] = [];
  const client = { product: (request: ProductRequest) => new Promise(resolve => answers.push(data => resolve({ ...request, data }))) } as unknown as DashboardClient;
  const plain = { id: 'joined-across', revision: '1' } as DashboardSource;
  const request: ProductRequest = { sourceId: plain.id, revision: plain.revision, product: 'synthetic-detail', params: { at: 1 } };
  const signal = () => new AbortController().signal;
  const followed = loadProduct(client, request, signal(), undefined, undefined, false, undefined, undefined, true);
  const ordinary = loadProduct(client, request, signal());
  await settle();
  assert.equal(answers.length, 1, 'one request for both');
  answers[0]({ detail: 1 });
  assert.deepEqual(await Promise.all([followed, ordinary]), [{ detail: 1 }, { detail: 1 }]);
});

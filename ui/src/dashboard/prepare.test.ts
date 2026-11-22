import { gatedHour } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { PrepareGate, baseClient, prepareGate, preparingClient } from './prepare.ts';
import type { PrepareFrames } from './prepare.ts';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import { loadProduct, peekProduct, productKey } from './products.ts';
import { busy, busyPreparing } from '../shell/busy.ts';

const request = (product: string, hour = 1) => ({ sourceId: 's', revision: 'r', product, params: { hour: gatedHour(hour) } }) as ProductRequest;
const flush = async () => { for (let turn = 0; turn < 6; turn++) await Promise.resolve(); };

function frames(): PrepareFrames & { run(): void } {
  let next: (() => void) | null = null;
  return { request: run => { next = run; return 1; }, cancel: () => { next = null; }, run() { const due = next; next = null; due?.(); } };
}

function transport() {
  const sent: { product: string; background: boolean; answer: () => void }[] = [];
  const send = (product: string) => (_signal: AbortSignal, background: boolean) =>
    new Promise<string>(resolve => sent.push({ product, background, answer: () => resolve(product) }));
  return { sent, send };
}

test('the lane holds one request in flight, sends none while the page has its own in flight, and the next once one ends', async () => {
  let pageIdle = false;
  const clock = frames();
  const gate = new PrepareGate(() => pageIdle, clock);
  const { sent, send } = transport();
  const signal = new AbortController().signal;
  const a = gate.ask(request('a'), send('a'), signal);
  const b = gate.ask(request('b'), send('b'), signal);
  assert.equal(sent.length, 0, 'the page is busy: nothing sent');
  assert.equal(gate.pending, 2);
  clock.run();
  assert.equal(sent.length, 0, 'still busy at the next frame');
  pageIdle = true;
  clock.run();
  assert.deepEqual(sent.map(item => item.product), ['a'], 'one at a time once the page is idle');
  assert.equal(sent[0].background, true);
  sent[0].answer();
  assert.equal(await a, 'a');
  await flush();
  assert.deepEqual(sent.map(item => item.product), ['a', 'b'], 'the next goes once the first ends');
  sent[1].answer();
  assert.equal(await b, 'b');
  await flush();
  assert.equal(gate.pending, 0);
});

test('what the page asks itself leaves the lane at once; what is left once the lane\'s askers go is sent; a cancelled ask leaves', async () => {
  const gate = new PrepareGate(() => false, frames());
  const { sent, send } = transport();
  const gone = new AbortController();
  const kept = gate.ask(request('kept'), send('kept'), new AbortController().signal);
  const left = gate.ask(request('left'), send('left'), gone.signal);
  const waited = gate.ask(request('waited'), send('waited'), new AbortController().signal);
  gate.promote(request('kept'));
  assert.deepEqual(sent.map(item => [item.product, item.background]), [['kept', false]], 'promoted: sent with the page busy, as the page\'s own');
  gone.abort();
  await assert.rejects(left, { name: 'AbortError' });
  assert.equal(gate.pending, 1, 'the cancelled one left the queue; the promoted one holds no lane slot');
  gate.release();
  assert.deepEqual(sent.map(item => item.product), ['kept', 'waited']);
  sent[0].answer(); sent[1].answer();
  assert.equal(await kept, 'kept');
  assert.equal(await waited, 'waited');
});

test('a prepared client reads into its base\'s caches, its requests apart from what the page waits on, and a page joining one waiting sends it', async () => {
  const asked: string[] = [];
  const counted: number[] = [];
  const base = {
    available: true,
    product<T>(query: ProductRequest): Promise<ProductReply<T>> {
      asked.push(query.product);
      const end = busy('requests');
      counted.push(busyPreparing());
      const reply = { sourceId: query.sourceId, revision: query.revision, product: query.product, data: { product: query.product } as T };
      return Promise.resolve().then(() => { end(); return reply; });
    },
  } as unknown as DashboardClient;
  const lane = preparingClient(base);
  assert.equal(baseClient(lane), base);
  assert.equal(prepareGate(lane), prepareGate(base));
  const prepared = request('prepared');
  assert.deepEqual(await loadProduct(lane, prepared, new AbortController().signal), { product: 'prepared' });
  assert.deepEqual(counted, [1], 'in flight as the lane\'s, never in the page\'s count');
  assert.deepEqual(peekProduct(base, prepared, productKey(prepared)), { product: 'prepared' }, 'held for the page\'s client');
  const again = await loadProduct(base, prepared, new AbortController().signal);
  assert.deepEqual(again, { product: 'prepared' });
  assert.deepEqual(asked, ['prepared'], 'the page reads what the lane read, asking nothing');
  const pageBusy = busy('requests');
  const joined = request('joined');
  const waiting = loadProduct(lane, joined, new AbortController().signal);
  await flush();
  assert.deepEqual(asked, ['prepared'], 'waits while the page has its own in flight');
  const page = loadProduct(base, joined, new AbortController().signal);
  await flush();
  assert.deepEqual(asked, ['prepared', 'joined'], 'sent once the page joined it');
  assert.deepEqual(await page, { product: 'joined' });
  assert.deepEqual(await waiting, { product: 'joined' });
  pageBusy();
});

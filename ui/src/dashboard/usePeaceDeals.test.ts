import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardClient, DashboardSource } from './client';
import { loadPreparedPeaceDeals } from './usePeaceDeals.ts';
import { deal, holder, reply } from '../widgets/territory-transfers/transfers.fixture.ts';

const source = { id: 'peace-source', revision: 'v2' } as DashboardSource;

test('peace and territory consumers read every prepared transfer without a branch scan', async () => {
  const from = holder(1, 'opaque/from', 100), to = holder(2, 'opaque/to', 101);
  const data = reply([deal('conference', Array.from({ length: 5000 }, (_, index) => ({ state: index + 1, from, to, before: 100, after: 101, vp: index === 0 ? '9223372036854775807' : '0' })))]);
  let calls = 0;
  const client = { preparedQuery: async (seen: DashboardSource, product: string, params: unknown) => {
    if (++calls > 1) throw new Error('Prepared peace exceeded its request bound');
    assert.equal(seen, source);
    assert.equal(product, 'events-page');
    assert.deepEqual(params, { branchId: '1', family: { kind: 'peace-deals' } });
    return data;
  }, product: () => { throw new Error('Unexpected peace branch scan'); } } as unknown as DashboardClient;
  const result = await loadPreparedPeaceDeals(client, source, '1', new AbortController().signal);
  assert.equal(result.deals[0].items.length, 5000);
  assert.equal(result.transfers.links[0].states.length, 5000);
  assert.equal(result.transfers.links[0].states.at(-1)?.state, 5000);
  assert.equal(calls, 1);
});

test('prepared peace refuses invalid replies and cancellation', async () => {
  await assert.rejects(loadPreparedPeaceDeals({ preparedQuery: async () => ({}) } as unknown as DashboardClient, source, '1', new AbortController().signal));
  const controller = new AbortController();
  await assert.rejects(loadPreparedPeaceDeals({ preparedQuery: async () => { controller.abort(); return reply([]); } } as unknown as DashboardClient,
    source, '1', controller.signal), { name: 'AbortError' });
});

test('unprepared peace outcomes stay pending until the shared query accepts coverage', async () => {
  const data = reply([deal('conference', [{ state: 1, from: holder(1, 'from', 100), to: holder(2, 'to', 101), before: 100, after: 101, vp: '0' }])]);
  let release!: () => void;
  const accepted = new Promise<void>(resolve => { release = resolve; });
  let queries = 0, statuses = 0, pending = 0, settled = false;
  const client = {
    preparedQuery: async () => {
      assert.ok(++queries <= 2, 'pending peace outcomes make at most two data requests');
      return queries === 1 ? null : data;
    },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 1, 'one accepted status completes peace outcomes');
      await accepted;
      return { ...request, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null };
    },
  } as unknown as DashboardClient;
  const loading = loadPreparedPeaceDeals(client, source, '1', new AbortController().signal,
    false, () => { pending++; }).then(value => { settled = true; return value; });
  try {
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(pending, 1);
    assert.equal(settled, false, 'null coverage must not become a successful empty peace reply');
    assert.equal(queries, 1);
  } finally { release(); }
  assert.equal((await loading).deals.length, 1);
  assert.deepEqual({ queries, statuses, pending }, { queries: 2, statuses: 1, pending: 1 });
});

import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { loadPreparedResearch } from './preparedResearch.ts';

const source = { id: 'research-source', revision: 'v2' } as DashboardSource;
const entry = (index: number) => ({ token: `repeat_${index}`, technologyHandle: index + 1, continuityStartHour: 0, level: index,
  firstObservedHour: index * 2, startedAfterHour: index * 2 - 1, lastObservedHour: index * 2, endedByHour: index * 2 + 1, outcome: 'completed' });

test('prepared research returns the complete selected-country row list without the product scan', async () => {
  const reply = { history: { coverage: [{ firstHour: 0, lastHour: 10_000 }], slots: [{ index: 0,
    availability: [{ firstHour: 0, lastHour: 10_000 }], entries: Array.from({ length: 5000 }, (_, index) => entry(index)), states: [], progress: [] }] },
    artwork: [], folders: [], technologyFolders: [], warnings: [] };
  let calls = 0;
  const client = { preparedQuery: async (seen: DashboardSource, product: string, params: unknown) => {
    if (++calls > 1) throw new Error('Prepared research exceeded its request bound');
    assert.equal(seen, source);
    assert.equal(product, 'events-page');
    assert.deepEqual(params, { branchId: '7', tag: 'opaque/tag', family: { kind: 'research' } });
    return reply;
  }, product: () => { throw new Error('Unexpected reference scan'); } } as unknown as DashboardClient;
  const result = await loadPreparedResearch(client, source, '7', 'opaque/tag', new AbortController().signal, false, () => {}, () => {});
  assert.equal(result.history.slots[0].entries.length, 5000);
  assert.equal(result.history.slots[0].entries.at(-1)?.token, 'repeat_4999');
  assert.equal(calls, 1);
});

test('research widgets showing one country share its complete reply', async () => {
  const reply = { history: { coverage: [{ firstHour: 0, lastHour: 10 }], slots: [] }, artwork: [], folders: [], technologyFolders: [], warnings: [] };
  let calls = 0;
  const client = { preparedQuery: async () => { calls++; return reply; } } as unknown as DashboardClient;
  const shown: unknown[] = [];
  const first = await loadPreparedResearch(client, source, '7', 'TAG', new AbortController().signal, false, () => {}, value => shown.push(value));
  const second = await loadPreparedResearch(client, source, '7', 'TAG', new AbortController().signal, false, () => {}, value => shown.push(value));
  assert.equal(calls, 1, 'the second widget takes the first one’s complete reply');
  assert.equal(second, first);
  assert.deepEqual(shown, [first, first]);
  await loadPreparedResearch(client, source, '7', 'OTHER', new AbortController().signal, false, () => {}, () => {});
  assert.equal(calls, 2, 'another country asks for its own');
});

test('prepared research refuses malformed data and cancelled replies', async () => {
  const controller = new AbortController();
  const client = { preparedQuery: async () => { controller.abort(); return {}; } } as unknown as DashboardClient;
  await assert.rejects(loadPreparedResearch(client, source, '7', 'opaque', controller.signal, false, () => {}, () => {}), { name: 'AbortError' });
  await assert.rejects(loadPreparedResearch({ preparedQuery: async () => ({}) } as unknown as DashboardClient,
    source, '7', 'opaque', new AbortController().signal, false, () => {}, () => {}), /research/i);
});

test('unprepared research stays pending until the shared query accepts coverage', async () => {
  const data = { history: { coverage: [{ firstHour: 100, lastHour: 101 }], slots: [] },
    artwork: [], folders: [], technologyFolders: [], warnings: [] };
  let release!: () => void;
  const accepted = new Promise<void>(resolve => { release = resolve; });
  let queries = 0, statuses = 0, pending = 0, settled = false;
  const client = {
    preparedQuery: async () => {
      assert.ok(++queries <= 2, 'pending research makes at most two data requests');
      return queries === 1 ? null : data;
    },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 1, 'one accepted status completes research');
      await accepted;
      return { ...request, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null };
    },
  } as unknown as DashboardClient;
  const loading = loadPreparedResearch(client, source, '7', 'opaque/tag', new AbortController().signal,
    false, () => { pending++; }, () => {}).then(value => { settled = true; return value; });
  try {
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(pending, 1);
    assert.equal(settled, false, 'null coverage must not become a successful empty research reply');
    assert.equal(queries, 1);
  } finally { release(); }
  assert.deepEqual((await loading).history.coverage, data.history.coverage);
  assert.deepEqual({ queries, statuses, pending }, { queries: 2, statuses: 1, pending: 1 });
});

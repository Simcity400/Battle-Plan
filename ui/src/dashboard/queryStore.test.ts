import test from 'node:test';
import assert from 'node:assert/strict';
import { setImmediate } from 'node:timers/promises';
import type { DashboardClient, DashboardQuery, DashboardResult } from './client';
import { QueryStore } from './queryStore.ts';

const query = (kind: DashboardQuery['kind'] = 'exact', hour = 100): DashboardQuery => ({
  sourceId: 'source', revision: 'revision', branchId: '1', countryIds: ['7'], firstHour: hour, lastHour: hour, kind, maxPoints: 1,
});
function result(request: DashboardQuery): DashboardResult {
  return { sourceId: request.sourceId, revision: request.revision, series: request.countryIds.map((countryId) => ({
    countryId, branchId: request.branchId, sessionId: '1', revision: request.revision, firstHour: request.firstHour, lastHour: request.lastHour,
    country: null, countries: [], points: [], coverage: { recordedHours: 0, gapHours: 1, absentHours: 0, unavailableHours: 0 },
    downsampled: false, sampleBoundary: 'unknown', valueScale: '100000', warnings: [],
  })) };
}
function harness(limit?: number) {
  const calls: { request: DashboardQuery; signal: AbortSignal; priority?: string; resolve: (value: DashboardResult) => void }[] = [];
  const client: DashboardClient = {
    available: true,
    openSource: async () => { throw new Error('not used'); },
    query: (request, signal, priority) => new Promise((resolve) => calls.push({ request, signal, priority, resolve })),
    releaseSource: async () => {}, setMapViews: async () => {}, onMapStates: async () => () => {},
  };
  return { store: new QueryStore(client, limit), calls };
}

test('normalized subscriptions share work and one closing consumer cannot cancel a sibling', async () => {
  const { store, calls } = harness();
  const entry = store.entry(query());
  assert.equal(store.entry({ ...query(), countryIds: ['7', '7'] }), entry);
  const release = store.retain(entry, () => {});
  const sibling = store.retain(entry, () => {});
  await setImmediate();
  assert.equal(calls.length, 1);
  release();
  await setImmediate();
  assert.equal(calls[0].signal.aborted, false);
  calls[0].resolve(result(calls[0].request));
  await setImmediate();
  assert.equal(entry.snapshot.status, 'ready');
  sibling();
  store.clear();
});

test('long histories leave capacity for a seek and publish independently', async () => {
  const { store, calls } = harness();
  const histories = [store.entry(query('history', 100)), store.entry(query('history', 101))];
  const releases = histories.map((entry) => store.retain(entry, () => {}));
  await setImmediate();
  assert.equal(calls.length, 1);
  const current = store.entry(query('exact', 102));
  releases.push(store.retain(current, () => {}));
  await setImmediate();
  assert.equal(calls.length, 2);
  assert.equal(calls[1].priority, 'current');
  calls[1].resolve(result(calls[1].request));
  await setImmediate();
  assert.equal(current.snapshot.status, 'ready');
  assert.equal(histories[0].snapshot.status, 'loading');
  calls[0].resolve(result(calls[0].request));
  await setImmediate();
  assert.equal(histories[0].snapshot.status, 'ready');
  assert.equal(calls.length, 3);
  calls[2].resolve(result(calls[2].request));
  releases.forEach((release) => release());
  store.clear();
});

test('cancelled completions cannot overwrite a retry and speculative requests keep their priority', async () => {
  const { store, calls } = harness();
  const entry = store.entry(query());
  const release = store.retain(entry, null);
  await setImmediate();
  assert.equal(calls[0].priority, 'prefetch');
  store.retry(entry);
  assert.equal(calls[0].signal.aborted, true);
  calls[0].resolve({ ...result(query()), revision: 'obsolete' });
  await setImmediate();
  assert.equal(entry.snapshot.status, 'loading');
  calls[1].resolve(result(query()));
  await setImmediate();
  assert.equal(entry.snapshot.result?.revision, 'revision');
  release();
  store.clear();
});

test('source, range and country mismatches fail locally instead of being labelled current', async () => {
  for (const corrupt of [
    (value: DashboardResult) => { value.sourceId = 'other'; },
    (value: DashboardResult) => { value.series[0].lastHour++; },
    (value: DashboardResult) => { value.series = []; },
  ]) {
    const { store, calls } = harness();
    const entry = store.entry(query());
    const release = store.retain(entry, () => {});
    await setImmediate();
    const response = result(query());
    corrupt(response);
    calls[0].resolve(response);
    await setImmediate();
    assert.equal(entry.snapshot.status, 'error');
    assert.equal(entry.snapshot.result, null);
    release();
    store.clear();
  }
});

test('a visible consumer promotes speculative work and ignores its cancelled completion', async () => {
  const { store, calls } = harness();
  const entry = store.entry(query());
  const prefetch = store.retain(entry, null);
  await setImmediate();
  const visible = store.retain(entry, () => {});
  await setImmediate();
  assert.equal(calls[0].signal.aborted, true);
  assert.equal(calls[1].priority, 'current');
  calls[1].resolve(result(query()));
  await setImmediate();
  calls[0].resolve({ ...result(query()), revision: 'obsolete' });
  await setImmediate();
  assert.equal(entry.snapshot.status, 'ready');
  assert.equal(entry.snapshot.result?.revision, 'revision');
  prefetch(); visible(); store.clear();
});

test('result admission preserves already visible data when the remaining budget is exhausted', async () => {
  const response = result(query());
  const bytes = JSON.stringify(response).length * 2 + 512;
  const { store, calls } = harness(bytes);
  const first = store.entry(query());
  const releaseFirst = store.retain(first, () => {});
  await setImmediate();
  calls[0].resolve(response);
  await setImmediate();
  assert.equal(first.snapshot.status, 'ready');
  const next = store.entry(query('exact', 101));
  const releaseNext = store.retain(next, () => {});
  await setImmediate();
  calls[1].resolve(result(calls[1].request));
  await setImmediate();
  assert.equal(next.snapshot.status, 'error');
  assert.equal(first.snapshot.status, 'ready');
  assert.equal(store.getUsage(), bytes);
  releaseFirst(); releaseNext(); store.clear();
});

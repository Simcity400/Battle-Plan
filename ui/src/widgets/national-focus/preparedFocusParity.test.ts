import test from 'node:test';
import assert from 'node:assert/strict';
import { gatherPreparedFocusParity } from './preparedFocusParity.ts';
import { loadPreparedFocusHour, loadPreparedFocusPage, loadPreparedFocusDetail } from './preparedFocusData.ts';
import type { FocusPreparedClient, PreparedFocusHour, PreparedFocusSnapshot } from './preparedFocusData.ts';
import type { DashboardSource } from '../../dashboard/client';

const source = { id: 'synthetic', revision: 'r' } as DashboardSource;
function fixture(size: number, fail = false) {
  const row: PreparedFocusHour = { schema: 6, slot: 1, countryId: 7, hour: 12, lastHour: 20, orderFirstHour: 4,
    current: 'active', continuous: null, paused: true, progressRaw: '9007199254740993001', dailyGainRaw: '7',
    completed: { first: '0', count: String(Math.ceil(size / 128)), rows: String(size) }, at: 12 };
  const calls: string[] = [];
  const counted = (product: string) => {
    calls.push(product);
    assert.ok(calls.length <= 1 + 6 * Math.ceil(size / 128), 'parity reads must stay proportional to the advertised page count');
  };
  const client = { preparedQuery: async (_source: unknown, product: string, params: any) => {
    counted(product);
    if (product === 'focus-static-text') return { texts: params.tokens.map((token: string) => ({ token, name: 'Observed', description: 'Static details', baseDurationDays: 7, icon: null })), warnings: [] };
    if (product === 'focus-hour') return { focus: { ...row, hour: Math.min(params.hour, 12), at: params.hour } };
    const start = Number(params.page) * 128;
    if (product === 'focus-order-page') return Array.from({ length: Math.min(128, size - start) - (fail ? 1 : 0) }, (_, i) =>
      ({ ordinal: start + i, token: 'repeat', completedByHour: (start + i) % 2 ? null : 8 }));
    throw new Error(product);
  }, product: async (request: any) => {
    counted(request.product);
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { warnings: [], failures: [],
      art: request.params.tokens.map((token: string) => ({ artwork: { token, name: 'Observed', description: null, baseDurationDays: null, icon: null }, reason: null })) } };
  } } as unknown as FocusPreparedClient;
  return { client, calls };
}
test('prepared parity gathers 5000 ordered rows and exact progress without legacy products', async () => {
  const { client, calls } = fixture(5000);
  const model = await gatherPreparedFocusParity(client, source, 'branch', 'opaque', 15, true, new AbortController().signal);
  assert.equal(model.complete, true); assert.equal(model.total, 5000);
  assert.deepEqual((model.rows[4999] as any), { key: '4999', index: 4999, item: { entry: { ordinal: 4999, token: 'repeat', completedByHour: null }, readHour: 4 } });
  assert.equal((model.current as any).progress.progress, 9007199254740993001n);
  assert.equal((model.artwork as any[]).length, 5000);
  assert.ok(calls.every(call => call.startsWith('focus-')));
  assert.ok(calls.length <= 241, '5000 ordered rows need at most six reads per page plus the initial point');
  assert.equal(calls.filter(call => call === 'focus-order-page').length, 40);
  console.log(JSON.stringify({ completions: 5000, requests: calls.length, requestLimit: 241 }));
});
test('whole campaign includes unfinished current with original ordinal', async () => {
  const { client } = fixture(129);
  const model = await gatherPreparedFocusParity(client, source, 'branch', 'opaque', 15, false, new AbortController().signal);
  assert.equal(model.total, 130);
  assert.deepEqual(model.rows[129], { key: '129', index: 129, item: { token: 'active', readHour: 15, paused: true, unfinished: true } });
  assert.equal((model.current as any).progress, null);
});
test('missing completion pages cannot certify parity', async () => {
  const { client } = fixture(129, true);
  await assert.rejects(gatherPreparedFocusParity(client, source, 'branch', 'opaque', 15, true, new AbortController().signal), /omitted rows/);
});

test('ready missing data yields an empty model; cancelled reads cannot certify', async () => {
  const client = { preparedQuery: async () => null } as unknown as FocusPreparedClient;
  const model = await gatherPreparedFocusParity(client, source, 'branch', 'opaque', 15, true, new AbortController().signal);
  assert.equal(model.total, 0); assert.equal(model.current, null);
  const controller = new AbortController(); controller.abort();
  await assert.rejects(gatherPreparedFocusParity(client, source, 'branch', 'opaque', 15, true, controller.signal), /cancelled/);
});

for (const size of [0, 47]) for (const moment of [false, true]) for (const openedOrdinal of size ? [undefined, 0] : [undefined]) test(`warm capture request trace: ${size} completions, moment=${moment}, opened=${openedOrdinal}`, async () => {
  const hour = 35, calls: { product: string; params: any }[] = [];
  let statuses = 0;
  const row: PreparedFocusHour = { schema: 6, slot: 1, countryId: 7, hour: 30, lastHour: 40, orderFirstHour: 10,
    current: size ? 'synthetic-active' : null, continuous: null, paused: true, progressRaw: '1000000', dailyGainRaw: null,
    completed: { first: '0', count: size ? '1' : '0', rows: String(size) }, at: hour };
  const token = (index: number) => index === size ? 'synthetic-active' : `synthetic-completed-${index}`;
  const art = (name: string, description: string | null = null) => ({ token: name, name: 'Observed', description, baseDurationDays: 7, icon: null });
  const client = { preparationStatus: async (request: any) => { statuses++; return { ...request, state: 'ready', sections: [] }; },
    preparedQuery: async (_source: unknown, product: string, params: any) => {
      calls.push({ product, params });
      if (product === 'focus-hour') return { focus: row };
      if (product === 'focus-order-page') return Array.from({ length: size }, (_, ordinal) => ({ ordinal, token: token(ordinal), completedByHour: null }));
      if (product === 'focus-static-text') return { texts: params.tokens.map((name: string) => art(name, 'Static details')), warnings: [] };
      throw new Error(product);
    },
    product: async (request: any) => {
      calls.push({ product: request.product, params: request.params });
      return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { warnings: [], failures: [],
        art: request.params.tokens.map((name: string) => ({ artwork: art(name, request.params.mode === 'text' ? 'Scoped details' : null), reason: null })) } };
    },
  } as unknown as FocusPreparedClient;
    calls.length = 0; statuses = 0;
    const signal = new AbortController().signal;
    const point = (await loadPreparedFocusHour(client, source, 'branch', 'opaque', hour, signal))!;
    const visible: PreparedFocusSnapshot = { ...await loadPreparedFocusPage(client, source, 'branch', 'opaque', hour, point, 0n, signal), hour: point, page: 0n };
    if (openedOrdinal !== undefined) visible.details = new Map([[openedOrdinal, (await loadPreparedFocusDetail(client, source, 'branch', 'opaque', point, visible.order[openedOrdinal], signal))!]]);
    const model = await gatherPreparedFocusParity(client, source, 'branch', 'opaque', hour, moment, signal, openedOrdinal === undefined ? [] : [openedOrdinal], visible);
    const counts = Object.fromEntries([...new Set(calls.map(call => call.product))].sort().map(product => [product, calls.filter(call => call.product === product).length]));
    // One numeric point; art at the completions' confirming hour and the
    // active focus's hour, and the opened row's description.
    const expected = size ? { 'focus-hour': 1, 'focus-order-page': 1, 'focus-static-text': 1,
      'focus-art': openedOrdinal === undefined ? 2 : 3 } : { 'focus-hour': 1 };
    console.log(JSON.stringify({ completions: size, moment, openedOrdinal: openedOrdinal ?? null, counts, statuses }));
    assert.deepEqual(counts, expected, 'parity reuses the settled numeric, presentation and opened-description reads');
    assert.equal(statuses, 0, 'accepted points do not start readiness polling');
    assert.equal(model.total, size + (size && !moment ? 1 : 0));
    assert.equal((model.staticText as any[]).length, size ? size + 1 : 0);
    if (openedOrdinal !== undefined) assert.equal((model.artwork as any[])[0].art.artwork.description, 'Scoped details');
});

test('a retained page at another displayed hour cannot supply the new parity model', async () => {
  const { client, calls } = fixture(129);
  const signal = new AbortController().signal;
  const row = (await loadPreparedFocusHour(client, source, 'branch', 'opaque', 15, signal))!;
  const visible = { ...await loadPreparedFocusPage(client, source, 'branch', 'opaque', 15, row, 1n, signal), hour: row, page: 1n };
  calls.length = 0;
  const model = await gatherPreparedFocusParity(client, source, 'branch', 'opaque', 16, true, signal, undefined, visible);
  assert.equal(model.total, 129);
  assert.equal(calls.filter(call => call === 'focus-order-page').length, 2);
});

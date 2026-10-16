import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardClient, DashboardSource } from './client.ts';
import { mergeReachedRows, preparedSeriesQuery, preparedWindowQuery } from './preparedSeries.ts';

const source = { id: 'source', revision: 'revision' } as DashboardSource;
test('pending terminal coverage retries advancing accepted progress', async () => {
  let queries = 0, statuses = 0, pauses = 0, pending = 0;
  const bounded = { series: [{ tag: 'opaque', summary: { buckets: [null] } }] };
  const client = {
    preparedQuery: async () => ++queries < 4 ? null : bounded,
    preparationStatus: async (request: object) => ({ ...request, state: ++statuses < 3 ? 'preparing' : 'ready',
      sections: [{ id: 'country-measure', label: 'Balances', state: 'preparing', done: String(6 + statuses), total: '9', error: null }], backgroundEnabled: true, bytes: '0', error: null }),
  } as unknown as DashboardClient;
  const result = await preparedSeriesQuery(client, source, 'series-summary', { to: 9 }, new AbortController().signal,
    false, () => { pending++; }, async () => { pauses++; await new Promise(resolve => setTimeout(resolve, 500)); });
  assert.equal(result, bounded);
  assert.equal(pending, 1);
  assert.equal(queries, 4, 'unchanged accepted extent does not reread summary pages');
  assert.equal(statuses, 3);
  assert.equal(pauses, 2, 'the shared loop retains the injected real wait');
});

test('instant waits stop repeated progress even when payload bytes grow', async () => {
  let queries = 0, statuses = 0, waits = 0;
  const client = {
    preparedQuery: async () => { queries++; return null; },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 2, 'stationary coverage must not request a third status');
      return { ...request, state: 'preparing', sections: [{ id: 'country-measure', label: 'Balances', state: 'preparing', done: '7', total: '9', error: null }],
        backgroundEnabled: true, bytes: String(statuses), error: null };
    },
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-summary', {}, new AbortController().signal, false, undefined,
    async () => { waits++; }), { message: 'Campaign preparation made no progress.' });
  assert.deepEqual({ queries, statuses, waits }, { queries: 2, statuses: 2, waits: 2 });
});

test('real waits permit unchanged accepted progress without rereading pages', async () => {
  let queries = 0, statuses = 0, waits = 0;
  const value = { states: [] };
  const client = {
    preparedQuery: async () => ++queries < 3 ? null : value,
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 3, 'only the advancing status can trigger another page read');
      return { ...request, state: 'preparing', sections: [{ id: 'country-measure', label: 'Balances', state: 'preparing',
        done: statuses < 3 ? '7' : '8', total: '9', error: null }], backgroundEnabled: true, bytes: '0', error: null };
    },
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'series-state', {}, new AbortController().signal, false, undefined,
    async () => { waits++; await new Promise(resolve => setTimeout(resolve, 500)); }), value);
  assert.deepEqual({ queries, statuses, waits }, { queries: 3, statuses: 3, waits: 2 });
});

test('explicit retry restarts failure while background is disabled, terminal states stop polling', async () => {
  let queries = 0, starts = 0;
  const client = {
    preparedQuery: async () => ++queries < 2 ? null : { states: [] },
    preparationStatus: async (request: object) => ({ ...request, state: starts ? 'ready' : 'failed', sections: [], backgroundEnabled: false, bytes: '0', error: 'failed' }),
    prepareSource: async (request: object) => { starts++; return { ...request, state: 'queued', sections: [], backgroundEnabled: true, bytes: '0', error: null }; },
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-state', {}, new AbortController().signal), /failed/);
  assert.equal(starts, 0);
  queries = 0;
  assert.deepEqual(await preparedSeriesQuery(client, source, 'series-state', {}, new AbortController().signal, true, undefined, async () => {}), { states: [] });
  assert.equal(starts, 1);
});

test('an unchanged status after a wait that took no time stops polling instead of spinning', async () => {
  let queries = 0, statuses = 0;
  const client = {
    preparedQuery: async () => { queries++; return null; },
    preparationStatus: async (request: object) => {
      // The old loop spun here forever; fail loudly instead of hanging the run.
      if (++statuses > 20) throw new Error('runaway status polling');
      return { ...request, state: 'preparing', sections: [{ id: 'country-measure', label: 'Balances', state: 'preparing', done: '7', total: '9', error: null }], backgroundEnabled: true, bytes: '0', error: null };
    },
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-summary', {}, new AbortController().signal, false, undefined, async () => {}),
    /made no progress/);
  assert.equal(statuses, 2);
  assert.equal(queries, 2);
});

test('source cancellation ends pending polling without another query', async () => {
  const controller = new AbortController();
  let queries = 0;
  const client = {
    preparedQuery: async () => { queries++; return null; },
    preparationStatus: async (request: object) => ({ ...request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null }),
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-summary', {}, controller.signal, false, undefined,
    async () => { controller.abort(); }), { name: 'AbortError' });
  assert.equal(queries, 2);
});


test('idle after Clear and disabled background never silently start balance work', async () => {
  for (const backgroundEnabled of [true, false]) {
    let starts = 0;
    const client = {
      preparedQuery: async () => null,
      preparationStatus: async (request: object) => ({ ...request, backgroundEnabled, state: 'idle', sections: [], bytes: '0', error: null }),
      prepareSource: async () => { starts++; throw new Error('unexpected automatic preparation'); },
    } as unknown as DashboardClient;
    await assert.rejects(preparedSeriesQuery(client, source, 'series-state', {}, new AbortController().signal), /Prepare this recording|Background preparation is off/);
    assert.equal(starts, 0);
  }
});

test('reply-budget paging reaches all 5000 requested series without a country cap', async () => {
  const series = Array.from({length:5000},(_,index) => ({id:String(index),spans:[{tag:`opaque ${index}`,firstHour:0,lastHour:5000,startsSegment:true}]}));
  type Page = { summaries: Record<string,number>; rows:string[] };
  let refused = 0,read = 0;
  const client = { preparedQuery: async (_source:unknown,_product:string,params:{within:Record<string,unknown>}) => {
    assert.ok(refused + read < 127, 'binary paging issues at most 127 requests for this fixture');
    if (Object.keys(params.within).length > 128) { refused++; throw new Error('Series summary exceeds the transport reply budget. Request a smaller viewport.'); }
    read++; return {summaries:Object.fromEntries(Object.keys(params.within).map(id => [id,1])),rows:Object.keys(params.within)};
  }} as unknown as DashboardClient;
  const result = await preparedWindowQuery(client,source,'autonomy',{branchId:'1'}, {from:0,to:5000,width:1000,series},new AbortController().signal,
    value => value as Page,(a,b) => ({summaries:{...a.summaries,...b.summaries},rows:[...a.rows,...b.rows]}));
  assert.ok(refused && read > 1); assert.equal(Object.keys(result.summaries).length,5000);
  assert.equal(read,64); assert.equal(refused,63);
  assert.ok(read+refused <= 127, 'each budget refusal splits into smaller requests');
  assert.deepEqual(new Set(result.rows),new Set(series.map(series => series.id)));
});

test('naval budget paging preserves family operands and every dated binding for 5000 countries', async () => {
  const series = Array.from({ length: 5000 }, (_, index) => ({ id: `mod country ${index}`, spans: [
    { tag: `old alias ${index}`, firstHour: 10, lastHour: 19, startsSegment: true },
    { tag: `mod country ${index}`, firstHour: 25, lastHour: 40, startsSegment: false },
  ] }));
  const family = { kind: 'caller kind', navalKind: 'both', attribution: 'sunkBy', period: 'month', top: 10 };
  const params = { branchId: '7', tags: ['selected holder'], parts: true, family };
  const requested = new Map<string, unknown>();
  let refused = 0, pages = 0;
  const client = { preparedQuery: async (_source: unknown, product: string, request: {
    branchId: string; tags: string[]; parts: boolean; family: object; from: number; to: number; width: number;
    within: Record<string, unknown>;
  }) => {
    assert.ok(refused + pages < 127, 'binary paging never repeats the requested pages');
    assert.equal(product, 'series-summary');
    assert.deepEqual(request.family, { ...family, kind: 'naval-losses' });
    assert.deepEqual([request.branchId, request.tags, request.parts, request.from, request.to, request.width],
      ['7', params.tags, true, 10, 40, 600]);
    if (Object.keys(request.within).length > 128) {
      refused++;
      throw new Error('Naval loss summary exceeds the transport reply budget. Request a smaller viewport.');
    }
    pages++;
    for (const [id, spans] of Object.entries(request.within)) {
      assert.equal(requested.has(id), false, 'a successful page supplies each country once');
      requested.set(id, spans);
    }
    return Object.keys(request.within);
  } } as unknown as DashboardClient;
  const result = await preparedWindowQuery(client, source, 'naval-losses', params,
    { from: 10, to: 40, width: 600, series }, new AbortController().signal,
    value => value as string[], (first, second) => [...first, ...second]);
  assert.ok(refused > 0 && pages > 1);
  assert.equal(refused + pages,127);
  assert.deepEqual(result, series.map(binding => binding.id));
  for (const binding of series) assert.deepEqual(requested.get(binding.id), binding.spans.map(span =>
    [span.tag, span.firstHour, span.lastHour, span.startsSegment]));
  assert.equal(requested.size, 5000);
  assert.equal(params.family, family);
  assert.equal(family.kind, 'caller kind', 'the helper does not mutate caller operands');
});

test('paged reached runs deduplicate supplying intervals while preserving breaks and every numeric field', () => {
  const head = {firstHour:10,lastHour:11,startsSegment:true,valueRaw:'9007199254740993'};
  const next = {firstHour:13,lastHour:15,startsSegment:true,valueRaw:'-2'};
  const a = [{tag:'mod country',slot:7,runs:[head]}],b = [{tag:'mod country',slot:7,runs:[head,next]}];
  assert.deepEqual(mergeReachedRows(a,b,row => JSON.stringify([row.tag,row.slot])),b);
  assert.throws(() => mergeReachedRows(a,[{...b[0],slot:8}],row => JSON.stringify([row.tag,row.slot])),/catalogue changed/);
});


test('partial replies publish immediately and replace only on changed accepted branch coverage', async () => {
  const values = [
    { coverage: { firstHour: 0, lastHour: 9, complete: false }, series: ['rough'] },
    { coverage: { firstHour: 0, lastHour: 19, complete: false }, series: ['part exact'] },
    { coverage: { firstHour: 0, lastHour: 29, complete: true }, series: ['exact'] },
  ];
  let reads = 0, statuses = 0;
  const published: unknown[] = [];
  const client = {
    preparedQuery: async () => { assert.ok(reads < 3, 'no extra data requests'); return values[reads++]; },
    preparationStatus: async (request: object) => ({ ...request, state: 'preparing', sections: [], bytes: '0', error: null,
      backgroundEnabled: true, coverage: { '1': { firstHour: 0, lastHour: statuses++ < 2 ? 9 : statuses === 3 ? 19 : 29, complete: statuses >= 4 } } }),
  } as unknown as DashboardClient;
  const result = await preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => { await new Promise(resolve => setTimeout(resolve, 500)); }, value => published.push(value));
  assert.equal(published[0], values[0]);
  assert.equal(result, values[2]);
  assert.equal(reads, 3);
  assert.equal(statuses, 4, 'unchanged branch coverage does not trigger another read');
});

test('a partial reply stays published when preparation fails and cancelled polling publishes no late data', async () => {
  const partial = { coverage: null, series: ['rough'] };
  const published: unknown[] = [];
  const client = { preparedQuery: async () => partial,
    preparationStatus: async (request: object) => ({ ...request, state: 'failed', sections: [], bytes: '0', error: 'Synthetic failure', backgroundEnabled: true }),
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-summary', {}, new AbortController().signal,
    false, undefined, async () => {}, value => published.push(value)), /Synthetic failure/);
  assert.deepEqual([...published], [partial]);
  const controller = new AbortController();
  client.preparationStatus = async request => { controller.abort(); return { ...request, state: 'preparing', sections: [], bytes: '0', error: null, backgroundEnabled: true }; };
  await assert.rejects(preparedSeriesQuery(client, source, 'series-summary', {}, controller.signal,
    false, undefined, async () => {}, value => published.push(value)), { name: 'AbortError' });
  assert.deepEqual(published, [partial, partial]);
});


test('a changed accepted status with the same partial reply stops after one re-read and one publication', { timeout: 2000 }, async () => {
  const partial = { coverage: { firstHour: 0, lastHour: 9, complete: false }, series: ['rough'] };
  let queries = 0, statuses = 0, publications = 0;
  const client = {
    preparedQuery: async () => { assert.ok(++queries <= 2, 'unchanged partial replies must stop the request loop'); return { ...partial }; },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 1, 'no further status requests after a no-progress reply');
      return { ...request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null,
        coverage: { '1': { firstHour: 0, lastHour: 19, complete: false } } };
    },
  } as unknown as DashboardClient;
  const result = await preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => {}, () => { publications++; });
  assert.deepEqual(result, partial);
  assert.equal(queries, 2);
  assert.equal(statuses, 1);
  assert.equal(publications, 1, 'identical partial replies never update React state again');
});

test('an immediate wait cannot spin Status or re-read unchanged accepted coverage as decoded counts grow', { timeout: 2000 }, async () => {
  const controller = new AbortController();
  const partial = { coverage: { firstHour: 0, lastHour: 9, complete: false }, series: ['rough'] };
  let queries = 0, statuses = 0, publications = 0;
  const client = {
    preparedQuery: async () => { assert.equal(++queries, 1, 'only the initial data request'); return partial; },
    preparationStatus: async (request: object) => {
      assert.equal(++statuses, 1, 'the instant-wait guard stops after unchanged branch coverage');
      return { ...request, state: 'preparing', backgroundEnabled: true, bytes: '0', error: null,
        sections: [{ id: 'country-measure', label: 'Balances', state: 'preparing', done: String(statuses), total: '663', error: null }],
        coverage: { '1': partial.coverage } };
    },
  } as unknown as DashboardClient;
  try {
    await assert.rejects(preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, controller.signal,
      false, undefined, async () => {}, () => { publications++; }), { message: 'Campaign preparation made no progress.' });
  } finally { controller.abort(); }
  assert.equal(queries, 1); assert.equal(statuses, 1); assert.equal(publications, 1);
});
test('5000-country pending coverage stops after two unchanged status replies with an instant wait', async () => {
  const params = { branchId: '1', tags: Array.from({ length: 5000 }, (_, index) => `mod country ${index} | α`),
    family: { kind: 'war-casualties' }, reads: [{ tag: '', hour: 9 }] };
  let queries = 0, statuses = 0;
  const client = {
    preparedQuery: async (_source: DashboardSource, product: string, actual: unknown) => {
      assert.equal(product, 'series-state'); assert.deepEqual(actual, params);
      assert.ok(++queries <= 2, 'unchanged coverage must not reread pages');
      return null;
    },
    preparationStatus: async (request: object) => {
      // The old loop spun here forever; fail loudly instead of hanging the run.
      if (++statuses > 20) throw new Error('runaway status polling');
      return { ...request, state: 'preparing', sections: [{ id: 'country-measure', label: 'Balances', state: 'preparing', done: '7', total: '9', error: null }], backgroundEnabled: true, bytes: '0', error: null };
    },
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-state', params, new AbortController().signal, false, undefined, async () => {}),
    /made no progress/);
  assert.equal(statuses, 2);
  assert.equal(queries, 2);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { preparedReplyData, type DashboardClient, type DashboardSource, type PreparedQueryOptions } from './client.ts';
import { validatePreparationReply } from '../shell/preparation/contract.ts';
import { isPreparedProduct, mergeReachedRows, preparedSeriesQuery, preparedWindowQuery } from './preparedSeries.ts';

const source = { id: 'source', revision: 'revision' } as DashboardSource;
// Provenance cases place commits eight seconds apart; cadence cases use their own clock.
const commitClock = (statuses: () => number) => () => statuses() * 8000 + Date.now();

test('focus publishes its first view then stops once that point has prepared dates', async () => {
  const first = { orderFirstHour: null, preparing: ['completedByHour'], coverage: { firstHour: 0, lastHour: 9, complete: false }, pending: [{ firstHour: 10, lastHour: 20 }] };
  const exact = { orderFirstHour: 10, coverage: { firstHour: 0, lastHour: 20, complete: false }, pending: [] };
  let queries = 0, statuses = 0;
  const client = {
    preparedQuery: async () => ++queries === 1 ? first : exact,
    preparationStatus: async (request: object) => {
      assert.equal(++statuses, 1);
      return { ...request, state: 'preparing', coverage: { '1': exact.coverage }, families: {}, sections: [], backgroundEnabled: true, bytes: '0', error: null };
    },
  } as unknown as DashboardClient;
  const published: unknown[] = [];
  const result = await preparedSeriesQuery(client, source, 'focus-hour', { branchId: '1', tag: 'MOD', hour: 20 }, new AbortController().signal,
    false, undefined, async () => { throw new Error('The exact point must not wait for the campaign'); }, value => published.push(value), commitClock(() => statuses));
  assert.equal(result, exact);
  assert.deepEqual(published, [first, exact]);
  assert.equal(queries, 2);
});

test('a useful frame reply keeps following its identity gate before the first coverage page exists', async () => {
  const cold = { coverage: { firstHour: 0, lastHour: -1, complete: false }, preparing: ['identities'], series: ['current values'] };
  const exact = { coverage: { firstHour: 0, lastHour: 29, complete: true }, series: ['exact history'] };
  let queries = 0, statuses = 0;
  const published: unknown[] = [];
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      options.gate?.(['identities']); return ++queries < 3 ? cold : exact;
    },
    preparationStatus: async (request: object) => familyStatus(request,
      ++statuses < 2 ? { identities: {} } : { identities: { '1': exact.coverage } }),
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'series-state', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => {}, value => published.push(value), commitClock(() => statuses)), exact);
  assert.deepEqual(published, [cold, exact]);
  assert.deepEqual({ queries, statuses }, { queries: 3, statuses: 2 });
});
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

test('a real wait measured a fraction short of its delay keeps polling unchanged progress', async () => {
  let queries = 0, statuses = 0, clock = 0;
  const value = { states: [] };
  const client = {
    preparedQuery: async () => ++queries < 3 ? null : value,
    preparationStatus: async (request: object) => {
      ++statuses;
      return { ...request, state: 'preparing', sections: [{ id: 'country-measure', label: 'Balances', state: 'preparing',
        done: statuses < 3 ? '7' : '8', total: '9', error: null }], backgroundEnabled: true, bytes: '0', error: null };
    },
  } as unknown as DashboardClient;
  // Coarsened clocks put a 500 ms timer at 499.9 ms.
  assert.equal(await preparedSeriesQuery(client, source, 'series-state', {}, new AbortController().signal, false, undefined,
    async () => { clock += 499.9; }, undefined, () => clock), value);
  assert.deepEqual({ queries, statuses }, { queries: 3, statuses: 3 });
});

test('explicit retry restarts failure while background is disabled, terminal states stop polling', async () => {
  let queries = 0, starts = 0;
  const client = {
    preparedQuery: async () => { queries++; return starts ? { states: [] } : null; },
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

test('mounted campaign windows publish the cold summary and replace it at accepted coverage', async () => {
  type Page = { coverage: { firstHour: number; lastHour: number; complete: boolean }; value: string };
  const cold: Page = { coverage: { firstHour: 0, lastHour: -1, complete: false }, value: 'sampled' };
  const exact: Page = { coverage: { firstHour: 0, lastHour: 20, complete: true }, value: 'prepared' };
  let reads = 0;
  const published: Page[] = [];
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      options.gate?.(['world-tension']); return ++reads === 1 ? cold : exact;
    },
    preparationStatus: async (request: object) => familyStatus(request, { 'world-tension': { '1': exact.coverage! } }),
  } as unknown as DashboardClient;
  const result = await preparedWindowQuery(client, source, 'world-tension', { branchId: '1' },
    { from: 0, to: 20, width: 10, series: [] }, new AbortController().signal,
    value => value as Page, (_first, second) => second, false, value => published.push(value));
  assert.deepEqual(published, [cold, exact]);
  assert.equal(result, exact);
  assert.equal(reads, 2);
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
    false, undefined, async () => { await new Promise(resolve => setTimeout(resolve, 500)); }, value => published.push(value), commitClock(() => statuses));
  assert.equal(published[0], values[0]);
  assert.equal(result, values[2]);
  assert.equal(reads, 3);
  assert.equal(statuses, 4, 'unchanged branch coverage does not trigger another read');
});

test('a partial reply stays published when preparation fails and cancelled polling publishes no late data', async () => {
  const partial = { coverage: { firstHour: 0, lastHour: -1, complete: false }, series: ['rough'] };
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


test('an unchanged partial reply stays mounted and follows the next accepted extent', async () => {
  const partial = { coverage: { firstHour: 0, lastHour: 9, complete: false }, series: ['prefix'] };
  const exact = { coverage: { firstHour: 0, lastHour: 29, complete: true }, series: ['exact'] };
  let queries = 0, statuses = 0;
  const published: unknown[] = [];
  const client = {
    preparedQuery: async () => { assert.ok(++queries <= 3); return queries < 3 ? { ...partial } : exact; },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 2);
      return { ...request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null,
        coverage: { '1': { firstHour: 0, lastHour: statuses === 1 ? 19 : 29, complete: statuses > 1 } } };
    },
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => {}, value => published.push(value), commitClock(() => statuses)), exact);
  assert.deepEqual(published, [partial, exact]);
  assert.deepEqual({ queries, statuses }, { queries: 3, statuses: 2 });
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

// A family prepared on its own moves while the hourly walk accepts nothing:
// the walk's coverage stays the same in every status.
const stalledWalk = { '1': { firstHour: 0, lastHour: 3, complete: false } };
const familyStatus = (request: object, families: Record<string, Record<string, { firstHour: number; lastHour: number; complete: boolean }>>) =>
  ({ ...request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null, coverage: stalledWalk, families });

test('mounted replies publish improved presentation without waiting for a longer forward prefix', async () => {
  const at = (lastHour: number, complete = false) => ({ firstHour: 0, lastHour, complete });
  const cases = [
    { product: 'series-state' as const, gate: ['balances'], before: { exact: null }, after: { exact: 90 } },
    { product: 'series-state' as const, gate: ['balances'], before: { pending: [{ firstHour: 10, lastHour: 99 }] },
      after: { pending: [{ firstHour: 10, lastHour: 39 }, { firstHour: 50, lastHour: 99 }] } },
    { product: 'series-summary' as const, gate: ['balances'], before: { approximate: [{ firstHour: 0, lastHour: 99 }] },
      after: { approximate: [{ firstHour: 0, lastHour: 39 }, { firstHour: 50, lastHour: 99 }] } },
    { product: 'series-state' as const, gate: ['balances'], before: { approximate: true }, after: { approximate: false } },
    { product: 'scoreboard' as const, gate: ['balances'], before: { preparing: ['first', 'second'] }, after: { preparing: ['second'] } },
  ];
  for (const { product, gate, before, after } of cases) {
    const values = [{ coverage: at(9), ...before }, { coverage: at(9), ...after },
      { coverage: at(9), ...after }, { coverage: at(99, true) }];
    const published: unknown[] = [];
    let reads = 0, statuses = 0;
    const client = {
      preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
        options.gate?.(gate);
        assert.ok(reads < values.length);
        return values[reads++];
      },
      preparationStatus: async (request: object) => {
        assert.ok(++statuses <= 3);
        return { ...familyStatus(request, Object.fromEntries(gate.map(name => [name,
          { '1': at(statuses * 30, statuses === 3) }]))),
          coverage: { '1': at(statuses === 3 ? 99 : statuses === 2 ? 19 : 9, statuses === 3) } };
      },
    } as unknown as DashboardClient;
    await preparedSeriesQuery(client, source, product, { branchId: '1' }, new AbortController().signal, false, undefined,
      async () => assert.deepEqual(published, [values[0], values[1]], `${product} must publish the newly answered hours immediately`),
      value => published.push(value), commitClock(() => statuses));
    assert.deepEqual(published, [values[0], values[1], values[3]], `${product} must suppress equivalent presentation`);
  }
});

test('mounted roster publishes new holders when only incomplete identity coverage advances', async () => {
  const at = (lastHour: number) => ({ firstHour: 0, lastHour, complete: false });
  const before = { preparedCoverage: at(48), selectionCoverage: at(48),
    pending: [{ firstHour: 49, lastHour: 71 }], preparing: ['countries'],
    countries: [{ tag: 'B', operatives: [{ id: '1' }] }] };
  const after = { ...before, selectionCoverage: at(96),
    countries: [...before.countries, { tag: 'C', operatives: [{ id: '2' }] }] };
  const values = [before, structuredClone(before), after];
  const published: unknown[] = [];
  const controller = new AbortController();
  let reads = 0, statuses = 0;
  const client = {
    preparedQuery: async (_source: DashboardSource, product: string, params: unknown, options: PreparedQueryOptions) => {
      assert.equal(product, 'operatives-roster');
      assert.deepEqual(params, { branchId: '1', tags: ['A'], firstHour: 48, lastHour: 71, includeDeparted: false });
      assert.ok(reads < values.length, 'one read per identity commit');
      options.gate?.(['identities', 'intelligence']);
      return values[reads++];
    },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 2);
      return familyStatus(request, { intelligence: { '1': at(48) }, identities: { '1': at(statuses === 1 ? 48 : 96) } });
    },
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'operatives-roster',
    { branchId: '1', tags: ['A'], firstHour: 48, lastHour: 71, includeDeparted: false }, controller.signal,
    false, undefined, async () => {
      assert.deepEqual(published, statuses === 1 ? [before] : [before, after],
        'publish the new rows before another family or forward coverage moves; suppress identical replies');
      if (statuses === 2) controller.abort();
    }, value => published.push(value), commitClock(() => statuses)), { name: 'AbortError' });
  assert.deepEqual({ reads, statuses }, { reads: 3, statuses: 2 });
  assert.deepEqual(published, [before, after]);
});

test('a pending reply waiting on a family prepared on its own is asked again when that family moves', { timeout: 2000 }, async () => {
  const exact = { coverage: { firstHour: 0, lastHour: 29, complete: true }, series: ['exact'] };
  let queries = 0, statuses = 0;
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      options.gate?.(['groups']);
      return ++queries < 3 ? null : exact;
    },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 2, 'the second status already moved the groups');
      return familyStatus(request, { groups: { '1': { firstHour: 0, lastHour: statuses === 1 ? 9 : 29, complete: statuses > 1 } } });
    },
  } as unknown as DashboardClient;
  // An instant wait: only a moved gate may ask again.
  assert.equal(await preparedSeriesQuery(client, source, 'country-groups', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => {}), exact);
  assert.deepEqual({ queries, statuses }, { queries: 3, statuses: 2 });
});

test('a partial reply waiting on a family is replaced as that family commits, not when the walk or another family moves', { timeout: 5000 }, async () => {
  const values = [
    { coverage: { firstHour: 0, lastHour: 9, complete: false }, series: ['prefix'] },
    { coverage: { firstHour: 0, lastHour: 19, complete: false }, series: ['longer prefix'] },
    { coverage: { firstHour: 0, lastHour: 29, complete: true }, series: ['exact'] },
  ];
  // The groups commit, stay (while the walk and the focus move), then complete.
  const groups = [values[1].coverage, values[1].coverage, values[2].coverage];
  let reads = 0, statuses = 0;
  const published: unknown[] = [];
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      assert.ok(reads < 3, 'one read per commit of the groups');
      if (reads === 2) assert.equal(statuses, 3, 'the exact reply is asked for once the groups complete');
      const value = values[reads++];
      options.gate?.(value.coverage.complete ? undefined : ['groups']);
      return value;
    },
    preparationStatus: async (request: object) => {
      const coverage = groups[statuses++];
      return { ...familyStatus(request, { groups: { '1': coverage }, focus: { '1': { firstHour: 0, lastHour: statuses, complete: false } } }),
        coverage: statuses === 2 ? { '1': { firstHour: 0, lastHour: 5, complete: false } } : stalledWalk };
    },
  } as unknown as DashboardClient;
  const result = await preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => { await new Promise(resolve => setTimeout(resolve, 520)); }, value => published.push(value), commitClock(() => statuses));
  assert.equal(result, values[2]);
  assert.deepEqual(published, values);
  assert.equal(reads, 3, 'unchanged groups coverage does not trigger another read');
  assert.equal(statuses, 3);
});

test('a reply waiting on several families is asked again when any of them moves', { timeout: 2000 }, async () => {
  const exact = { coverage: { firstHour: 0, lastHour: 29, complete: true }, series: ['exact'] };
  let queries = 0, statuses = 0;
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      options.gate?.(['groups', 'walk']);
      return ++queries < 4 ? null : exact;
    },
    preparationStatus: async (request: object) => {
      statuses++;
      // First the groups move, then only the walk.
      return { ...familyStatus(request, { groups: { '1': { firstHour: 0, lastHour: statuses === 1 ? 9 : 19, complete: false } } }),
        coverage: { '1': { firstHour: 0, lastHour: statuses < 3 ? 3 : 7, complete: false } } };
    },
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'event-feed', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => {}), exact);
  assert.deepEqual({ queries, statuses }, { queries: 4, statuses: 3 });
});

test('a prepared reply hands over its gate, and malformed gates and family coverage are refused', () => {
  const reply = { sourceId: 'source', revision: 'revision', product: 'country-groups', data: null };
  const seen: (readonly string[] | undefined)[] = [];
  const options = { signal: new AbortController().signal, gate: (entries: readonly string[] | undefined) => { seen.push(entries); } };
  assert.equal(preparedReplyData({ ...reply, gate: ['groups', 'walk'] }, options), null);
  assert.equal(preparedReplyData(reply, options), null);
  assert.deepEqual(seen, [['groups', 'walk'], undefined]);
  assert.throws(() => preparedReplyData({ ...reply, gate: [3] as unknown as string[] }, options), /gate is malformed/);
  const request = { sourceId: 'source', revision: 'revision', requestId: 'status' };
  const status = { ...request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null, coverage: {} };
  const families = { groups: { '1': { firstHour: 0, lastHour: 9, complete: false } } };
  assert.deepEqual(validatePreparationReply({ ...status, families } as never, request).families, families);
  assert.throws(() => validatePreparationReply({ ...status, families: { groups: { '1': { firstHour: 9, lastHour: 0, complete: false } } } } as never, request),
    /Invalid preparation progress/);
});

test('a partial reply following several families keeps polling while one moves and their joint coverage does not', { timeout: 5000 }, async () => {
  const at = (lastHour: number, complete = false) => ({ firstHour: 0, lastHour, complete });
  const values = [{ coverage: at(9), losses: ['prefix'] }, { coverage: at(9), losses: ['prefix'] },
    { coverage: at(19), losses: ['longer prefix'] }, { coverage: at(29, true), losses: ['exact'] }];
  // The casualties move first; the joint coverage waits for the wars.
  const families = [{ casualties: at(9), wars: at(9) }, { casualties: at(19), wars: at(9) },
    { casualties: at(19), wars: at(19) }, { casualties: at(29, true), wars: at(29, true) }];
  let reads = 0, statuses = 0;
  const published: unknown[] = [];
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      const value = values[reads++];
      options.gate?.(value.coverage.complete ? undefined : ['casualties', 'wars']);
      return value;
    },
    preparationStatus: async (request: object) => {
      const moved = families[Math.min(statuses++, families.length - 1)];
      return familyStatus(request, Object.fromEntries(Object.entries(moved).map(([name, coverage]) => [name, { '1': coverage }])));
    },
  } as unknown as DashboardClient;
  const result = await preparedSeriesQuery(client, source, 'casualty-breakdown', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => { await new Promise(resolve => setTimeout(resolve, 520)); }, value => published.push(value), commitClock(() => statuses));
  assert.equal(result, values[3]);
  assert.deepEqual(published, [values[0], values[2], values[3]]);
  assert.equal(reads, 4);
});


test('cold capture coverage stays separate and a joint gate keeps polling before its first accepted hour', { timeout: 3000 }, async () => {
  const empty = { coverage: [], preparedCoverage: { firstHour: 0, lastHour: -1, complete: false }, pending: [{ firstHour: 0, lastHour: 29 }] };
  const complete = { coverage: [], preparedCoverage: { firstHour: 0, lastHour: 29, complete: true }, pending: [] };
  let reads = 0, statuses = 0;
  const published: unknown[] = [];
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      options.gate?.(['intelligence', 'identities']);
      return ++reads < 3 ? empty : complete;
    },
    preparationStatus: async (request: object) => {
      const at = { firstHour: 0, lastHour: ++statuses === 1 ? 9 : 29, complete: statuses > 1 };
      return familyStatus(request, { intelligence: { '1': at }, identities: statuses === 1 ? {} : { '1': at } });
    },
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'agency-comparison', { selection: { branchId: '1' } }, new AbortController().signal,
    false, undefined, async () => {}, value => published.push(value), commitClock(() => statuses)), complete);
  assert.equal(reads, 3);
  assert.deepEqual(published, [empty, complete]);
});

test('a joint gate that shrinks to wars retains polling until wars complete', async () => {
  const at = (lastHour: number, complete = false) => ({ firstHour: 0, lastHour, complete });
  const values = [{ coverage: at(9), rows: ['prefix'] }, { coverage: at(9), rows: ['prefix'] },
    { coverage: at(19), rows: ['longer'] }, { coverage: at(29, true), rows: ['exact'] }];
  let reads = 0, statuses = 0;
  const published: unknown[] = [];
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      assert.ok(reads < values.length);
      options.gate?.(reads === 0 ? ['casualties', 'wars'] : ['wars']);
      return values[reads++];
    },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 3);
      return familyStatus(request, { casualties: { '1': at(29, true) },
        wars: { '1': at(statuses === 1 ? 9 : statuses === 2 ? 19 : 29, statuses === 3) } });
    },
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'casualty-breakdown', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => {}, value => published.push(value), commitClock(() => statuses)), values[3]);
  assert.deepEqual(published, [values[0], values[2], values[3]]);
  assert.deepEqual({ reads, statuses }, { reads: 4, statuses: 3 });
});


test('shown replies refresh at 2 then 4 then 8 seconds after replies; completion bypasses the delay', async () => {
  let time = 0;
  const asks: { at: number; refresh: boolean }[] = [];
  const replies: number[] = [], published: unknown[] = [];
  const extent = () => ({ firstHour: 0, lastHour: Math.floor(time / 500), complete: time >= 16000 });
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      asks.push({ at: time, refresh: options.refresh ?? false });
      options.gate?.(['balances']);
      time += 300; // The interval starts after this work and delivery, not at dispatch.
      replies.push(time);
      return { coverage: extent(), series: [time] };
    },
    preparationStatus: async (request: object) => familyStatus(request, { balances: { '1': extent() } }),
  } as unknown as DashboardClient;
  const value = await preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => { time += 500; }, value => published.push(value), () => time);
  assert.deepEqual(asks.map(ask => ask.at), [0, 2300, 6600, 14900, 16200]);
  assert.deepEqual(asks.map(ask => ask.refresh), [false, true, true, true, true]);
  assert.deepEqual(asks.slice(1, 4).map((ask, index) => ask.at - replies[index]), [2000, 4000, 8000]);
  assert.equal(asks[4].at - replies[3], 1000, 'the completed gate did not wait eight seconds');
  assert.equal(published.length, 5);
  assert.equal(value, published[4]);
});

test('no first view and new navigation, ranges and selections ask immediately', async () => {
  let time = 0, statuses = 0, reads = 0;
  const asks: { at: number; refresh: boolean; params: unknown }[] = [];
  const exact = { coverage: { firstHour: 0, lastHour: 29, complete: true }, states: [] };
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, params: unknown, options: PreparedQueryOptions) => {
      asks.push({ at: time, refresh: options.refresh ?? false, params });
      options.gate?.(['balances']);
      return ++reads < 3 ? null : exact;
    },
    preparationStatus: async (request: object) => familyStatus(request,
      { balances: { '1': { firstHour: 0, lastHour: ++statuses, complete: false } } }),
  } as unknown as DashboardClient;
  await preparedSeriesQuery(client, source, 'series-state', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => { time += 500; }, () => {}, () => time);
  assert.deepEqual(asks.map(ask => ask.at), [0, 0, 500]);
  for (const [product, params] of [
    ['series-next-change', { branchId: '1', key: 'ArrowRight' }],
    ['series-summary', { branchId: '1', from: 10, to: 20 }],
    ['series-state', { branchId: '1', tags: ['new selection'] }],
  ] as const) {
    await preparedSeriesQuery(client, source, product, params, new AbortController().signal,
      false, undefined, async () => { throw new Error('user actions must not wait'); }, () => {}, () => time);
    assert.equal(asks.at(-1)!.at, 500);
    assert.deepEqual(asks.at(-1)!.params, params);
  }
  assert.ok(asks.every(ask => !ask.refresh));
});

test('a joint gate completes only when every supplier on the selected branch completes', async () => {
  let time = 0, statuses = 0, reads = 0;
  const at = (complete: boolean) => ({ firstHour: 0, lastHour: 9, complete });
  const first = { coverage: at(false), pending: [{ firstHour: 10, lastHour: 29 }] };
  const final = { coverage: at(true), pending: [] };
  const client = {
    preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
      options.gate?.(['a', 'b']);
      return ++reads === 1 ? first : final;
    },
    preparationStatus: async (request: object) => {
      statuses++;
      assert.equal(reads, 1, 'the incomplete joint gate bypassed the refresh interval');
      return familyStatus(request, { a: { '1': at(true), '2': at(false) },
        b: { '1': at(statuses > 1), '2': at(false) } });
    },
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'series-state', { selection: { branchId: '1' } },
    new AbortController().signal, false, undefined, async () => { time += 500; }, () => {}, () => time), final);
  assert.deepEqual({ statuses, reads, time }, { statuses: 2, reads: 2, time: 500 });
});


test('a moved gate cannot spin an instant wait while a shown reply is throttled', async () => {
  let reads = 0, statuses = 0;
  const partial = { coverage: { firstHour: 0, lastHour: 9, complete: false }, series: [] };
  const client = {
    preparedQuery: async () => { reads++; return partial; },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 2);
      return { ...request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null,
        coverage: { '1': { firstHour: 0, lastHour: 19, complete: false } } };
    },
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-state', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => {}, () => {}, () => 0), /made no progress/);
  assert.deepEqual({ reads, statuses }, { reads: 2, statuses: 2 }, 'one final read drains the deferred gate before the guard stops polling');
});


test('stopping preparation delivers a throttled exact focus point before raising its terminal state', async () => {
  for (const state of ['failed', 'cancelled', 'idle'] as const) {
    let time = 0, reads = 0, statuses = 0;
    const first = { coverage: { firstHour: 0, lastHour: 9, complete: false },
      pending: [{ firstHour: 10, lastHour: 20 }], preparing: ['completedByHour'] };
    const exact = { coverage: { firstHour: 0, lastHour: 20, complete: false }, pending: [], completedByHour: 20 };
    const published: unknown[] = [];
    const client = {
      preparedQuery: async () => { assert.ok(++reads <= 2); return reads === 1 ? first : exact; },
      preparationStatus: async (request: object) => {
        statuses++;
        return { ...request, state: time >= 1000 ? state : 'preparing', sections: [], backgroundEnabled: true,
          bytes: '0', error: time >= 1000 ? `Stopped: ${state}` : null,
          coverage: { '1': time >= 500 ? exact.coverage : first.coverage } };
      },
    } as unknown as DashboardClient;
    assert.equal(await preparedSeriesQuery(client, source, 'focus-hour', { branchId: '1', hour: 20 },
      new AbortController().signal, false, undefined, async () => { time += 500; },
      value => published.push(value), () => time), exact);
    assert.deepEqual(published, [first, exact]);
    assert.deepEqual({ reads, statuses, time }, { reads: 2, statuses: 3, time: 1000 });
  }
});

test('stopping preparation publishes its latest prefix before retaining the terminal error', async () => {
  let time = 0, reads = 0;
  const first = { coverage: { firstHour: 0, lastHour: 9, complete: false }, rows: ['first'] };
  const latest = { coverage: { firstHour: 0, lastHour: 20, complete: false }, rows: ['latest'] };
  const published: unknown[] = [];
  const client = {
    preparedQuery: async () => ++reads === 1 ? first : latest,
    preparationStatus: async (request: object) => ({ ...request, state: time >= 1000 ? 'failed' : 'preparing',
      coverage: { '1': time >= 500 ? latest.coverage : first.coverage }, sections: [],
      backgroundEnabled: true, bytes: '0', error: time >= 1000 ? 'Stopped after commit' : null }),
  } as unknown as DashboardClient;
  await assert.rejects(preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' },
    new AbortController().signal, false, undefined, async () => { time += 500; },
    value => published.push(value), () => time), /Stopped after commit/);
  assert.equal(reads, 2);
  assert.deepEqual(published, [first, latest]);
});

test('paused preparation waits without failing and the read finishes once it resumes', async () => {
  let time = 0, reads = 0;
  const first = { coverage: { firstHour: 0, lastHour: 9, complete: false }, rows: ['first'] };
  const exact = { coverage: { firstHour: 0, lastHour: 20, complete: true }, rows: ['exact'] };
  const published: unknown[] = [];
  const client = {
    preparedQuery: async () => ++reads === 1 ? first : exact,
    preparationStatus: async (request: object) => ({ ...request, state: time < 2000 ? 'paused' : 'ready', pauseReason: time < 2000 ? 'newerRecording' : null,
      coverage: { '1': time < 2000 ? first.coverage : exact.coverage }, sections: [], backgroundEnabled: true, bytes: '0', error: null }),
  } as unknown as DashboardClient;
  assert.equal(await preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, new AbortController().signal,
    false, undefined, async () => { time += 500; }, value => published.push(value), () => time), exact);
  assert.deepEqual(published, [first, exact]);
  assert.equal(reads, 2, 'an unchanged pause asks for nothing new');
});

test('null replies receive one final ask on a terminal state even when the named gate stays empty', async () => {
  for (const state of ['failed', 'cancelled', 'idle'] as const) {
    let reads = 0, statuses = 0;
    const client = {
      preparedQuery: async (_source: DashboardSource, _product: string, _params: unknown, options: PreparedQueryOptions) => {
        reads++; options.gate?.(['army']); return null;
      },
      preparationStatus: async (request: object) => ({ ...request, state: ++statuses === 1 ? 'preparing' : state,
        families: { army: {} }, sections: [], backgroundEnabled: true, bytes: '0', error: `Stopped: ${state}` }),
    } as unknown as DashboardClient;
    await assert.rejects(preparedSeriesQuery(client, source, 'army-size', { branchId: '1' },
      new AbortController().signal, false, undefined, async () => {}, () => {}), /Stopped:/);
    assert.deepEqual({ reads, statuses }, { reads: 3, statuses: 2 });
  }
});

test('status and wait failures drain deferred progress, while source cancellation discards it', async () => {
  for (const exit of ['status', 'wait', 'cancel'] as const) {
    let time = 0, reads = 0;
    const controller = new AbortController();
    const first = { coverage: { firstHour: 0, lastHour: 9, complete: false }, rows: ['first'] };
    const latest = { coverage: { firstHour: 0, lastHour: 20, complete: false }, rows: ['latest'] };
    const published: unknown[] = [];
    const client = {
      preparedQuery: async () => ++reads === 1 ? first : latest,
      preparationStatus: async (request: object) => {
        if (time >= 1000 && exit === 'status') throw new Error('Status failed');
        return { ...request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null,
          coverage: { '1': time >= 500 ? latest.coverage : first.coverage } };
      },
    } as unknown as DashboardClient;
    const operation = preparedSeriesQuery(client, source, 'series-summary', { branchId: '1' }, controller.signal,
      false, undefined, async () => {
        time += 500;
        if (time >= 1000 && exit === 'wait') throw new Error('Wait failed');
        if (time >= 1000 && exit === 'cancel') controller.abort();
      }, value => published.push(value), () => time);
    await assert.rejects(operation, exit === 'cancel' ? { name: 'AbortError' } : /failed/);
    assert.equal(reads, exit === 'cancel' ? 1 : 2);
    assert.deepEqual(published, exit === 'cancel' ? [first] : [first, latest]);
  }
});

test('faction-state follows only its independent family gate from a cold prefix', async () => {
  assert.equal(isPreparedProduct('faction-state'), true);
  const cold = { accounts: [], coverage: { firstHour: 100, lastHour: 99, complete: false }, pending: [{ firstHour: 100, lastHour: 101 }] };
  const exact = { accounts: [], coverage: { firstHour: 100, lastHour: 101, complete: true }, pending: [] };
  let queries = 0;
  const client = {
    preparedQuery: async (_source: unknown, product: string, params: unknown, options: PreparedQueryOptions) => {
      assert.equal(product, 'faction-state'); assert.deepEqual(params, { view: 'influence', branchId: '1', faction: '9' });
      options.gate?.(['faction_state']); return ++queries === 1 ? cold : exact;
    },
    preparationStatus: async (request: object) => familyStatus(request, { faction_state: { '1': exact.coverage } }),
  } as unknown as DashboardClient;
  const published: unknown[] = [];
  assert.equal(await preparedSeriesQuery(client, source, 'faction-state', { view: 'influence', branchId: '1', faction: '9' }, new AbortController().signal,
    false, undefined, async () => {}, value => published.push(value)), exact);
  assert.deepEqual(published, [cold, exact]);
});

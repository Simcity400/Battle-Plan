import assert from 'node:assert/strict';
import test from 'node:test';
import { forgetProducts, loadCountryGroups, loadProduct, loadTransientProduct, productKey, replyLengths } from './products.ts';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';

const request: ProductRequest = { sourceId: 's', revision: 'v', product: 'national-focus', params: { branchId: '1', tag: 'GER' } };

test('ace windows and transient portraits read prepared data while oracle requests remain separate', async () => {
  const source = { id: 'aces', revision: '1' } as DashboardSource;
  const window = { sourceId: source.id, revision: source.revision, product: 'air-aces',
    params: { branchId: '1', view: 'window', window: 7 } };
  const art = { ...window, product: 'air-ace-portraits', params: { branchId: '1', hour: 100, ids: ['1', '2'] } };
  const calls: string[] = [];
  const client = { preparedQuery: async (_source: unknown, product: string) => { calls.push(product); return { prepared: true }; },
    product: async (request: ProductRequest) => ({ ...request, data: { oracle: true } }) } as unknown as DashboardClient;
  assert.deepEqual(await loadProduct(client, window, new AbortController().signal, undefined, source), { prepared: true });
  assert.deepEqual(await loadProduct(client, window, new AbortController().signal), { oracle: true });
  for (let attempt = 0; attempt < 2; attempt++)
    assert.deepEqual(await loadTransientProduct(client, art, new AbortController().signal, source), { prepared: true });
  assert.deepEqual(calls, ['air-aces', 'air-ace-portraits', 'air-ace-portraits']);
});

test('ledger widgets share prepared event replies and availability never loads the legacy scan', async () => {
  const source = { id: 'ledger', revision: '1' } as import('./client').DashboardSource;
  const request = { sourceId: source.id, revision: source.revision, product: 'military-air-ledger',
    params: { branchId: '1', family: 'airLosses', availability: true } };
  let prepared = 0;
  const client = { preparedQuery: async (_source: unknown, product: string, params: unknown) => {
    assert.equal(product, request.product); assert.deepEqual(params, request.params);
    prepared++; return { available: true };
  }, product: async () => { throw new Error('request-time ledger scan'); } } as unknown as DashboardClient;
  const replies = await Promise.all(Array.from({ length: 24 }, () =>
    loadProduct(client, request, new AbortController().signal, undefined, source)));
  assert.equal(prepared, 1);
  assert.ok(replies.every(reply => reply === replies[0]));
  assert.deepEqual(replies[0], { available: true });
  await assert.rejects(loadProduct(client, request, new AbortController().signal, undefined,
    { ...source, revision: 'stale' }), /does not match/);
});

test('raids and air-superiority requests use prepared adapters and retain the oracle route', async () => {
  const source = { id: 'air-series', revision: '1' } as DashboardSource;
  const calls: string[] = [];
  const client = { preparedQuery: async (_source: unknown, product: string, params: unknown) => {
    calls.push(product); return { params };
  }, product: async (request: ProductRequest) => ({ ...request, data: { oracle: true } }) } as unknown as DashboardClient;
  for (const product of ['raids', 'air-superiority']) {
    const request = { sourceId: source.id, revision: source.revision, product, params: { branchId: '1', view: 'choices', firstHour: 100, lastHour: 123 } };
    assert.deepEqual(await loadProduct(client, request, new AbortController().signal, undefined, source), { params: request.params });
    assert.deepEqual(await loadProduct(client, request, new AbortController().signal), { oracle: true });
  }
  assert.deepEqual(calls, ['raids', 'air-superiority']);
});

test('air2 prepared replies retain 5000 actors with bounded requests through the real product loaders', async () => {
  const actors = Array.from({ length: 5000 }, (_, index) => `opaque actor ${index}`);
  const source = { id: 'air2-scale', revision: '1' } as DashboardSource;
  for (const [product, params] of [
    ['military-air-ledger', { branchId: '1', family: 'airLosses' }],
    ['air-aces', { branchId: '1', view: 'window', window: 0 }],
    ['raids', { branchId: '1', view: 'tally', tags: actors, top: null }],
    ['air-superiority', { branchId: '1', view: 'choices' }],
  ] as const) {
    let queries = 0, statuses = 0;
    const client = { preparedQuery: async (_source: unknown, actual: string, reached: unknown) => {
      assert.ok(++queries <= 2, `${product} exceeded its request bound`);
      assert.equal(actual, product); assert.deepEqual(reached, params);
      return queries === 1 ? null : { actors };
    }, preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 1, `${product} exceeded its Status bound`);
      return { ...request, state: 'preparing', backgroundEnabled: true, bytes: '1', error: null,
        sections: [{ id: product, label: product, state: 'preparing', done: '1', total: '2', error: null }] };
    }, product: async () => { throw new Error('request-time scan'); } } as unknown as DashboardClient;
    const input = { sourceId: source.id, revision: source.revision, product, params };
    const [first, second] = await Promise.all([0, 1].map(() =>
      loadProduct<{ actors: string[] }>(client, input, new AbortController().signal, undefined, source)));
    assert.deepEqual(first.actors, actors); assert.equal(first, second);
    assert.equal(queries, 2); assert.equal(statuses, 1);
    await loadProduct(client, input, new AbortController().signal, undefined, source);
    assert.equal(queries, 2, 'the completed 5000-actor reply is shared');
  }
  let portraits = 0;
  const client = { preparedQuery: async (_source: unknown, product: string, params: { ids: string[] }) => {
    assert.ok(++portraits <= 1, 'ace portraits exceeded their request bound');
    assert.equal(product, 'air-ace-portraits'); assert.deepEqual(params.ids, actors);
    return { actors };
  }, product: async () => { throw new Error('request-time portrait scan'); } } as unknown as DashboardClient;
  const reply = await loadTransientProduct<{ actors: string[] }>(client, { sourceId: source.id, revision: source.revision,
    product: 'air-ace-portraits', params: { branchId: '1', hour: 100, ids: actors } }, new AbortController().signal, source);
  assert.deepEqual(reply.actors, actors); assert.equal(portraits, 1);
});

test('casualty pair availability and selections use prepared windows and share requests', async () => {
  const source = { id: 'pairs', revision: '1' } as DashboardSource;
  let queries = 0;
  const client = { product: async () => { throw new Error('request-time pair scan'); },
    preparedQuery: async (_source: DashboardSource, product: string, params: unknown) => {
      assert.equal(product, 'casualty-pairs'); queries++; return { params };
    } } as unknown as DashboardClient;
  for (const window of [null, 0, 2]) {
    const params = { branchId: '1', tags: ['mod country | α', 'other country'], between: true, window };
    const input = { sourceId: source.id, revision: source.revision, product: 'casualty-pairs', params };
    const values = await Promise.all(Array.from({ length: 12 }, () => loadProduct(client, input, new AbortController().signal, undefined, source)));
    assert.deepEqual(values[0], { params });
    assert.ok(values.every(value => value === values[0]));
  }
  assert.equal(queries, 3);
});

test('equipment-loss viewports use prepared routing and preserve exact reached dates', async () => {
  const source = { id: 'equipment-losses', revision: '1' } as DashboardSource;
  const params = { selection: { branchId: '1', tag: 'mod country', landSource: 'destruction', country: { slot: 4999, countryId: 104999, originHour: 10 } },
    from: 100, to: 200, width: 800, reads: [10, 100, 175, 200], headHour: 175, detailHour: 175 };
  let queries = 0;
  const client = { product: async () => { throw new Error('request-time loss scan'); },
    preparedQuery: async (_source: DashboardSource, product: string, reached: unknown) => {
      queries++; assert.equal(product, 'equipment-losses'); assert.deepEqual(reached, params); return { reached };
    } } as unknown as DashboardClient;
  const input = { sourceId: source.id, revision: source.revision, product: 'equipment-losses', params };
  await Promise.all(Array.from({ length: 17 }, () => loadProduct(client, input, new AbortController().signal, undefined, source)));
  assert.equal(queries, 1);
});

test('forces use prepared coverage without scanning and keep all 5000 requested countries', async () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `opaque/${index}`);
  const input = { ...request, product: 'army-size', params: { branchId: '1', tags, window: 0 } };
  const source = { id: 's', revision: 'v' } as DashboardSource;
  let pages = 0, scans = 0, statuses = 0;
  const client = {
    product: async (request: ProductRequest) => { scans++; return { ...request, data: { oracle: true } }; },
    preparedQuery: async (_source: DashboardSource, product: string, params: { tags: string[] }) => {
      assert.equal(product, 'army-size'); assert.deepEqual(params.tags, tags);
      assert.ok(++pages <= 2, '5000-country query exceeded its request bound');
      return pages === 1 ? null : { tags };
    },
    preparationStatus: async (request: { sourceId: string; revision: string; requestId: string }) => {
      assert.ok(++statuses <= 1, '5000-country query exceeded its Status bound');
      return { ...request, backgroundEnabled: true, state: 'preparing', sections: [{ id: 'army-size', label: 'Army size', state: 'preparing', done: '1', total: '2', error: null }], bytes: '1', error: null };
    },
  } as unknown as DashboardClient;
  const signal = new AbortController().signal;
  assert.deepEqual(await loadProduct(client, input, signal), { oracle: true });
  const prepared = await loadProduct<{ tags: string[] }>(client, input, signal, undefined, source);
  assert.deepEqual(prepared.tags, tags);
  assert.equal(scans, 1); assert.equal(pages, 2); assert.equal(statuses, 1);
});

test('identical product requests share one native round trip and forgetting a source reloads', async () => {
  let calls = 0;
  const client = { product: async (input: ProductRequest) => { calls++; return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: { calls } }; } } as unknown as DashboardClient;
  const results = await Promise.all(Array.from({ length: 24 }, () => loadProduct<{ calls: number }>(client, request, new AbortController().signal)));
  assert.equal(calls, 1);
  assert.ok(results.every(value => value === results[0]));
  await loadProduct(client, { ...request, params: { branchId: '1', tag: 'ENG' } }, new AbortController().signal);
  assert.equal(calls, 2);
  await loadProduct(client, request, new AbortController().signal);
  assert.equal(calls, 2);
  forgetProducts(client, 's');
  await loadProduct(client, request, new AbortController().signal);
  assert.equal(calls, 3);
});

test('replies for another recording or product are rejected and payload validators run once per load', async () => {
  const wrong = { product: async () => ({ sourceId: 'other', revision: 'v', product: 'national-focus', data: {} }) } as unknown as DashboardClient;
  await assert.rejects(loadProduct(wrong, request, new AbortController().signal), /does not match/);
  let validated = 0;
  const good = { product: async (input: ProductRequest) => ({ ...input, data: { n: 1 } }) } as unknown as DashboardClient;
  const validate = (data: unknown) => { validated++; return data as { n: number }; };
  const [first, second] = await Promise.all([
    loadProduct(good, request, new AbortController().signal, validate),
    loadProduct(good, request, new AbortController().signal, validate),
  ]);
  assert.equal(validated, 1);
  assert.equal(first, second);
  assert.equal(productKey(request), productKey({ ...request, params: { branchId: '1', tag: 'GER' } }));
  assert.notEqual(productKey(request), productKey({ ...request, revision: 'v2' }));
});

test('the cache sizes a reply by the length its transport received, validated or not', async () => {
  for (const validate of [undefined, (data: unknown) => data]) {
    let calls = 0;
    const client = { product: async (input: ProductRequest) => {
      calls++;
      const reply = { ...input, data: { small: true } };
      // Received at the whole budget: once counted, it cannot be kept.
      replyLengths.set(reply, 64 * 1024 * 1024);
      return reply;
    } } as unknown as DashboardClient;
    await loadProduct(client, request, new AbortController().signal, validate);
    await loadProduct(client, request, new AbortController().signal, validate);
    assert.equal(calls, 2);
  }
});


test('prepared country groups share the product cache, content revisions and source release', async () => {
  const source = { id: 'groups', revision: '1' } as import('./client').DashboardSource;
  const data = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], countryIdentities: [], unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  let queries = 0;
  const client = { preparedQuery: async (_source: unknown, product: string) => { assert.equal(product, 'country-groups'); queries++; return data; },
    product: async () => { throw new Error('request-time group scan'); } } as unknown as DashboardClient;
  const load = () => loadCountryGroups(client, source, '1', new AbortController().signal);
  const replies = await Promise.all(Array.from({ length: 51 }, load));
  assert.equal(queries, 1);
  assert.ok(replies.every(reply => reply === replies[0]));
  const { contentRevision } = await import('./contentRevision.ts');
  contentRevision(client).observe({ game: 'modded-game', user: 'user', contentReady: true });
  await load();
  assert.equal(queries, 2);
  forgetProducts(client, source.id);
  await load();
  assert.equal(queries, 3);
});

test('prepared group consumers cancel independently and failures remain retryable', async () => {
  const source = { id: 'cancel-groups', revision: '1' } as import('./client').DashboardSource;
  let cancelled = false;
  const client = { preparedQuery: async (_source: unknown, _product: string, _params: unknown, options: { signal: AbortSignal }) => {
    await new Promise<void>((_resolve, reject) => options.signal.addEventListener('abort', () => { cancelled = true; reject(new DOMException('cancelled', 'AbortError')); }, { once: true }));
  } } as unknown as DashboardClient;
  const a = new AbortController(), b = new AbortController();
  const first = loadCountryGroups(client, source, '1', a.signal);
  const second = loadCountryGroups(client, source, '1', b.signal);
  await Promise.resolve(); await Promise.resolve();
  a.abort();
  await assert.rejects(first, { name: 'AbortError' });
  assert.equal(cancelled, false);
  b.abort();
  await assert.rejects(second, { name: 'AbortError' });
  await Promise.resolve();
  assert.equal(cancelled, true);
  await assert.rejects(loadCountryGroups({ preparedQuery: async () => { throw new Error('refused'); } } as unknown as DashboardClient, source, '1', new AbortController().signal), /refused/);
});


test('special projects preserve complete replies and selection roots through prepared routing and the shared cache', async () => {
  const projects = Array.from({ length: 5000 }, (_, id) => ({ id: String(id) }));
  const request: ProductRequest = { sourceId: 'special-source', revision: '2', product: 'special-projects',
    params: { branchId: '1', tag: 'opaque-alias', selectionTags: ['original-root'] } };
  let queries = 0;
  const client = { preparedQuery: async (source: { id: string; revision: string }, product: string, params: unknown) => {
    assert.deepEqual(source, { id: request.sourceId, revision: request.revision });
    assert.equal(product, request.product);
    assert.deepEqual(params, request.params);
    if (++queries > 1) throw new Error('Prepared special projects exceeded its request bound');
    return { projects };
  }, product: async () => { throw new Error('request-time special-project walk'); } } as unknown as DashboardClient;
  const replies = await Promise.all(Array.from({ length: 24 }, () => loadProduct<{ projects: typeof projects }>(client, request, new AbortController().signal)));
  assert.equal(queries, 1);
  assert.ok(replies.every(reply => reply === replies[0]));
  assert.equal(replies[0].projects.length, 5000);
  assert.deepEqual(replies[0].projects[4999], { id: '4999' });
});

test('land manifest, windows, ranking and course use prepared queries through the shared cache', async () => {
  for (const product of ['land-battles-manifest', 'land-battles', 'land-battles-ranking', 'land-battle-course']) {
    let queries = 0;
    const client = { preparedQuery: async (_source: unknown, actual: string) => { assert.equal(actual, product); queries++; return { rows: [] }; },
      product: async () => { throw new Error('request-time land scan'); } } as unknown as DashboardClient;
    const input = { sourceId: 'land', revision: '1', product, params: { branchId: '1', window: 0 } };
    await Promise.all([loadProduct(client, input, new AbortController().signal), loadProduct(client, input, new AbortController().signal)]);
    assert.equal(queries, 1);
  }
});


test('naval windows, manifest, ranking and drawn details use preparation without a history request', async () => {
  for (const product of ['naval-battles', 'naval-battles-manifest', 'naval-battles-ranking', 'naval-battle-details']) {
    const client = { preparedQuery: async (_source: unknown, actual: string) => { assert.equal(actual, product); return { rows: [] }; },
      product: async () => { throw new Error('request-time naval scan'); } } as unknown as DashboardClient;
    await loadProduct(client, { sourceId: 'naval', revision: '1', product, params: { branchId: '1' } }, new AbortController().signal);
  }
});


test('division log, availability and filtered totals use prepared pages', async () => {
  for (const product of ['division-log', 'division-log-manifest', 'division-log-totals']) {
    const client = { preparedQuery: async (_source: unknown, actual: string) => { assert.equal(actual, product); return { created: 0, lost: 0, transferred: 0 }; },
      product: async () => { throw new Error('request-time division scan'); } } as unknown as DashboardClient;
    await loadProduct(client, { sourceId: 'division', revision: '1', product, params: { branchId: '1' } }, new AbortController().signal);
  }
});

test('an explicit prepared product retry resumes a failed prefix while automatic preparation is off', async () => {
  let starts = 0;
  const client = { preparedQuery: async () => starts ? { created: 1 } : null,
    preparationStatus: async (input: object) => ({ ...input, state: starts ? 'ready' : 'failed', backgroundEnabled: false, sections: [], bytes: '0', error: 'preparation stopped' }),
    prepareSource: async (input: object) => { starts++; return { ...input, state: 'queued', backgroundEnabled: false, sections: [], bytes: '0', error: null }; },
  } as unknown as DashboardClient;
  const input = { sourceId: 'retry-division', revision: '1', product: 'division-log-totals', params: { branchId: '1' } };
  await assert.rejects(loadProduct(client, input, new AbortController().signal), /preparation stopped/);
  assert.equal(starts, 0);
  assert.deepEqual(await loadProduct(client, input, new AbortController().signal, undefined, undefined, true), { created: 1 });
  assert.equal(starts, 1);
});

test('prepared subjects share validated replies and never start a request-time scan', async () => {
  const source = { id: 'subjects', revision: '1' } as import('./client').DashboardSource;
  const request = { sourceId: source.id, revision: source.revision, product: 'subjects', params: { branchId: '1' } };
  let calls = 0, validations = 0;
  const client = { preparedQuery: async (actual: typeof source, product: string, params: unknown) => {
    assert.equal(actual, source); assert.equal(product, 'subjects'); assert.deepEqual(params, request.params);
    calls++; return { branchId: '1', levels: [], runs: [], unobserved: [], warnings: [] };
  }, product: async () => { throw new Error('request-time scan'); } } as unknown as DashboardClient;
  const { validateSubjectReply } = await import('../widgets/subjects/subjectsData.ts');
  const validate = (data: unknown) => { validations++; return validateSubjectReply(data); };
  const replies = await Promise.all([0, 1].map(() => loadProduct(client, request, new AbortController().signal, validate, source)));
  assert.equal(replies[0], replies[1]); assert.equal(calls, 1); assert.equal(validations, 1);
});

test('prepared wars separate structural metadata from complete reached-hour requests', async () => {
  const source={id:'prepared-wars',revision:'1'} as DashboardSource;
  const calls:unknown[]=[];
  const client={preparedQuery:async (actual:DashboardSource,product:string,params:unknown)=>{
    assert.equal(actual,source);assert.equal(product,'wars');calls.push(params);
    return {branchId:'1',available:true,viewpoints:[],countryNames:[],wars:[],unobserved:[],capitulationsUnobserved:[],warnings:[]};
  },product:async ()=>{throw new Error('request-time scan');}} as unknown as DashboardClient;
  const {validateWarsReply}=await import('../widgets/wars/warsData.ts');
  const structural={sourceId:source.id,revision:source.revision,product:'wars',params:{branchId:'1',viewpointTag:null}};
  const exact={...structural,params:{...structural.params,reads:[{hour:100},{hour:120}]}};
  const signal=new AbortController().signal;
  await Promise.all([loadProduct(client,structural,signal,validateWarsReply,source),loadProduct(client,exact,signal,validateWarsReply,source)]);
  await loadProduct(client,exact,signal,validateWarsReply,source);
  assert.deepEqual(calls,[structural.params,exact.params]);
});


test('territory pages use shared prepared polling and never invoke the whole-history oracle', async()=>{
  const source={id:'s',revision:'v'} as DashboardSource;
  const tags=Array.from({length:5000},(_,index)=>`opaque/${index}`);
  let queries=0;
  const client={product:async()=>{throw Error('request-time territory walk');},preparedQuery:async(_source:DashboardSource,kind:string,params:{tags:string[]})=>{
    queries++;assert.equal(kind,'series-state');assert.deepEqual(params.tags,tags);return {tags};
  }} as unknown as DashboardClient;
  const input={...request,product:'series-state',params:{branchId:'1',family:{kind:'territory',measure:'states',rule:'owned'},tags,reads:[{hour:100}]}};
  const reply=await loadProduct<{tags:string[]}>(client,input,new AbortController().signal,undefined,source);
  assert.deepEqual(reply.tags,tags);
  assert.equal(queries,1);
});


test('occupied windows, manifests and recording-wide owners use prepared polling without a replay walk', async () => {
  const source = { id: 'territory-products', revision: '1' } as DashboardSource;
  const client = { product: async () => { throw new Error('request-time occupation or ownership walk'); },
    preparedQuery: async (_source: DashboardSource, product: string) => ({ product }),
  } as unknown as DashboardClient;
  for (const product of ['occupied-states', 'occupied-states-manifest', 'land-owners']) {
    assert.deepEqual(await loadProduct(client, { sourceId: source.id, revision: source.revision, product, params: {} }, new AbortController().signal, undefined, source), { product });
  }
});

test('all intelligence products share prepared coverage polling and preserve request operands', async () => {
  const source = { id: 's', revision: 'v' } as DashboardSource;
  for (const product of ['espionage-log', 'operative-placements', 'operatives-roster', 'agency-comparison', 'decryption']) {
    const params = { branchId: '1', tags: Array.from({ length: 5000 }, (_, index) => `mod/${index}`), firstHour: 100, lastHour: 123 };
    let reads = 0, statuses = 0;
    const client = {
      product: async () => { throw new Error('an intel widget scanned its branch'); },
      preparedQuery: async (actualSource: DashboardSource, actualProduct: string, actualParams: unknown) => {
        assert.ok(++reads <= 2, `${product} repeated a prepared read without progress`);
        assert.equal(actualSource, source); assert.equal(actualProduct, product); assert.equal(actualParams, params);
        return reads === 1 ? null : { product, params };
      },
      preparationStatus: async (input: unknown) => {
        assert.ok(++statuses <= 1, `${product} repeated a status request without progress`);
        return { ...(input as object), backgroundEnabled: true,
          state: 'preparing', sections: [{ id: product, label: product, state: 'preparing', done: '1', total: '2', error: null }], bytes: '1', error: null };
      },
    } as unknown as DashboardClient;
    const actual = await loadProduct(client, { ...request, product, params }, new AbortController().signal, undefined, source);
    assert.deepEqual(actual, { product, params }); assert.equal(reads, 2); assert.equal(statuses, 1);
  }
});

test('construction windows and spending endpoints use prepared requests with separate oracle caches', async () => {
  const source = { id: 'construction', revision: 'v' } as DashboardSource;
  let scans = 0, queries = 0;
  const client = {
    product: async (input: ProductRequest) => { scans++; return { ...input, data: { oracle: true } }; },
    preparedQuery: async (_source: DashboardSource, product: string, params: unknown) => { queries++; return { product, params }; },
  } as unknown as DashboardClient;
  for (const product of ['construction-log-manifest', 'construction-log', 'construction-spending']) {
    const input = { sourceId: source.id, revision: source.revision, product, params: { branchId: '1', window: 0, hours: [10, 20], tag: 'opaque / tag' } };
    const signal = new AbortController().signal;
    assert.deepEqual(await loadProduct(client, input, signal, undefined, source), { product, params: input.params });
    assert.deepEqual(await loadProduct(client, input, signal), { oracle: true });
  }
  assert.equal(queries, 3);
  assert.equal(scans, 3);
});

test('resource and trade viewports use prepared coverage and keep their oracle caches distinct', async () => {
  const source = { id: 'economy', revision: '1' } as DashboardSource;
  const calls: string[] = [];
  const client = { preparedQuery: async (_source: DashboardSource, product: string, params: unknown) => { calls.push(product); return params; },
    product: async (request: ProductRequest) => ({ ...request, data: { oracle: true } }) } as unknown as DashboardClient;
  for (const product of ['resource-balance', 'trade-flows', 'production-spending']) {
    const request = { sourceId: source.id, revision: source.revision, product, params: { branchId: '1', from: 100.5, to: 120.5, width: 25, hours: [112] } };
    assert.deepEqual(await loadProduct(client, request, new AbortController().signal, undefined, source), request.params);
    assert.deepEqual(await loadProduct(client, request, new AbortController().signal), { oracle: true });
  }
  assert.deepEqual(calls, ['resource-balance', 'trade-flows', 'production-spending']);
});

test('5000 resource countries and trade participants keep prepared request counts bounded', { timeout: 10_000 }, async () => {
  const source = { id: 'economy-scale', revision: '1' } as DashboardSource;
  const tags = Array.from({ length: 5000 }, (_, index) => `opaque country/${index}`);
  let resourceReads = 0, tradeReads = 0, statuses = 0, scans = 0;
  const client = {
    product: async () => { scans++; throw new Error('request-time economy scan'); },
    preparationStatus: async () => { statuses++; throw new Error('ready pages must not poll'); },
    preparedQuery: async (actual: DashboardSource, product: string, params: { tag?: string; tags?: string[] }) => {
      assert.equal(actual, source);
      if (product === 'resource-balance') {
        assert.ok(++resourceReads <= tags.length, 'at most one prepared read per requested country');
        return { tag: params.tag };
      }
      assert.equal(product, 'trade-flows');
      assert.ok(++tradeReads <= 1, 'all participants share one trade page');
      assert.deepEqual(params.tags, tags);
      return { tags: params.tags };
    },
  } as unknown as DashboardClient;
  for (const tag of tags) {
    const request = { sourceId: source.id, revision: source.revision, product: 'resource-balance', params: { branchId: '1', tag, hours: [10] } };
    const replies = await Promise.all([0, 1].map(() => loadProduct<{ tag: string }>(client, request, new AbortController().signal, undefined, source)));
    assert.equal(replies[0], replies[1], 'identical country reads share the real product cache');
    assert.equal(replies[0].tag, tag);
  }
  const request = { sourceId: source.id, revision: source.revision, product: 'trade-flows', params: { branchId: '1', tags, hours: [10] } };
  const replies = await Promise.all([0, 1].map(() => loadProduct<{ tags: string[] }>(client, request, new AbortController().signal, undefined, source)));
  assert.equal(replies[0], replies[1]);
  assert.deepEqual(replies[0].tags, tags);
  assert.equal(resourceReads, 5000);
  assert.equal(tradeReads, 1);
  assert.equal(statuses, 0);
  assert.equal(scans, 0);
});

test('casualty breakdown manifests and windows share prepared requests without either legacy scan', async () => {
  const source = { id: 'breakdown', revision: '1' } as DashboardSource;
  let queries = 0;
  const client = { product: async () => { throw new Error('request-time casualty or war scan'); },
    preparedQuery: async (_source: DashboardSource, product: string, params: unknown) => {
      assert.equal(product, 'casualty-breakdown'); queries++; return { params };
    } } as unknown as DashboardClient;
  for (const window of [null, 0, 2]) {
    const params = { branchId: '1', tag: 'mod holder | α', window };
    const input = { sourceId: source.id, revision: source.revision, product: 'casualty-breakdown', params };
    const values = await Promise.all(Array.from({ length: 17 }, () => loadProduct(client, input, new AbortController().signal, undefined, source)));
    assert.deepEqual(values[0], { params });
    assert.ok(values.every(value => value === values[0]));
  }
  assert.equal(queries, 3);
});

test('casualty pair availability and 5000-country selections share one request per prepared window', async () => {
  const source = { id: 'pairs', revision: '1' } as DashboardSource;
  const tags = Array.from({ length: 5000 }, (_, index) => `mod country ${index} | α`);
  let queries = 0;
  const client = { product: async () => { throw new Error('request-time pair scan'); },
    preparedQuery: async (_source: DashboardSource, product: string, params: unknown) => {
      assert.equal(product, 'casualty-pairs');
      assert.ok(++queries <= 3, 'three selected windows require at most three requests');
      return { params };
    } } as unknown as DashboardClient;
  for (const window of [null, 0, 2]) {
    const params = { branchId: '1', tags, between: true, window };
    const input = { sourceId: source.id, revision: source.revision, product: 'casualty-pairs', params };
    const values = await Promise.all(Array.from({ length: 12 }, () => loadProduct(client, input, new AbortController().signal, undefined, source)));
    assert.deepEqual(values[0], { params });
    assert.ok(values.every(value => value === values[0]));
  }
  assert.equal(queries, 3);
});

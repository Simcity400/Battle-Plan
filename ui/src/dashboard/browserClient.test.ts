import assert from 'node:assert/strict';
import test from 'node:test';

test('a page going away names itself, the owner of everything it opened or imported, so the bridge lets go of it all', async () => {
  const handlers = new Map<string, (event: { persisted: boolean }) => void>(), beacons: Blob[] = [];
  const owners: unknown[] = [];
  Object.assign(globalThis, { window: globalThis, addEventListener: (type: string, handler: (event: { persisted: boolean }) => void) => handlers.set(type, handler) });
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { sendBeacon: (_url: string, body: Blob) => beacons.push(body) } });
  // The file input the picker opens, choosing one recording.
  const listeners = new Map<string, () => void>();
  const input = { files: [{ name: 'Imported.battleplan', size: 1 }], addEventListener: (type: string, listener: () => void) => listeners.set(type, listener),
    click: () => listeners.get('change')!(), remove: () => {} };
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { createElement: () => input, body: { append: () => {} } } });
  globalThis.fetch = (async (url: string, init: RequestInit) => {
    if (url === '/api/recordings/import') {
      owners.push((init.headers as Record<string, string>)['X-Recording-Page']);
      return { ok: true, json: async () => ({ path: 'imports/Imported.battleplan' }) };
    }
    const body = JSON.parse(String(init.body));
    if (body.type === 'open') owners.push(body.page);
    const reply = body.type === 'open' ? { id: `lease-${body.path}`, revision: 'r', metadata: {} } : {};
    return { ok: true, text: async () => JSON.stringify(reply) };
  }) as unknown as typeof fetch;
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  await client.openSource('a', AbortSignal.timeout(1000));
  await client.openSource('b', AbortSignal.timeout(1000));
  await client.releaseSource('lease-a');
  assert.equal(await client.chooseRecording!(), 'imports/Imported.battleplan');
  handlers.get('pagehide')!({ persisted: true });
  assert.equal(beacons.length, 0, 'a cached page retains what it holds for Back navigation');
  handlers.get('pagehide')!({ persisted: false });
  assert.deepEqual(beacons.map(beacon => beacon.type), ['application/json'], 'the bridge accepts JSON MIME, including on page unload');
  const [gone] = await Promise.all(beacons.map(async beacon => JSON.parse(await beacon.text())));
  assert.equal(gone.type, 'pageGone');
  assert.ok(typeof gone.page === 'string' && gone.page.length > 0);
  assert.deepEqual(owners, [gone.page, gone.page, gone.page], 'every open and import names the page that leaves');
});

const tick = () => new Promise<void>(resolve => setImmediate(resolve));
interface PendingHttp {
  body: { type: string; sourceId?: string; refresh?: boolean; request?: import('./client').ProductRequest };
  signal: AbortSignal;
  done: boolean;
  finish: (data?: unknown, ok?: boolean) => void;
  fail: (error: Error) => void;
}
function pendingTransport() {
  const previous = globalThis.fetch, requests: PendingHttp[] = [];
  globalThis.fetch = (async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body));
    let resolve!: (text: string) => void, reject!: (error: Error) => void;
    const text = new Promise<string>((yes, no) => { resolve = yes; reject = no; });
    const response = { ok: true, text: () => text };
    const request: PendingHttp = { body, signal: init.signal as AbortSignal, done: false,
      finish: (data = {}, ok = true) => {
        request.done = true; response.ok = ok;
        resolve(JSON.stringify((body.type === 'product' || body.type === 'preparedQuery') && ok ? { ...body.request, data } : data));
      },
      fail: error => { request.done = true; reject(error); },
    };
    requests.push(request);
    return response;
  }) as unknown as typeof fetch;
  return { requests, restore: () => { globalThis.fetch = previous; }, drain: async () => {
    for (let pass = 0; pass < 10; pass++) { for (const request of requests) if (!request.done) request.finish(); await tick(); }
  } };
}
const query = (product: string, sourceId = 'scheduler-source'): import('./client').ProductRequest => ({
  sourceId, revision: 'v1', product, params: { branchId: '1', tag: 'Synthetic country', hour: 100, tokens: ['synthetic-focus'] },
});

test('actual browser transport reserves focus capacity while five histories wait and eventually returns every history', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const { INTERACTIVE_PRODUCTS } = await import('../generated/dashboard.ts');
  assert.ok(INTERACTIVE_PRODUCTS.includes('focus-artwork'));
  const transport = pendingTransport();
  const histories = Array.from({ length: 5 }, () => client.product(query('national-focus'), new AbortController().signal));
  try {
    await tick();
    assert.equal(transport.requests.length, 3, 'ordinary reads leave the interactive connections free before fetch');
    const focus = client.product<{ name: string }>(query('focus-artwork'), new AbortController().signal);
    await tick();
    const http = transport.requests.find(request => request.body.request?.product === 'focus-artwork')!;
    assert.ok(http, 'dated text reaches fetch without waiting for a history reply');
    http.finish({ name: 'Localized title immediately available' });
    assert.equal((await focus).data.name, 'Localized title immediately available');
    assert.ok(transport.requests.filter(request => request.body.request?.product === 'national-focus').every(request => !request.done));
    await transport.drain();
    assert.equal((await Promise.all(histories)).length, 5, 'admission queues work without omitting histories');
    assert.equal(transport.requests.filter(request => request.body.request?.product === 'national-focus').length, 5);
  } finally { await transport.drain(); transport.restore(); }
});

test('actual transport cancels queued reads without fetch, holds an aborted body slot, and preserves independent text after an image refusal', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const transport = pendingTransport();
  const controllers = Array.from({ length: 4 }, () => new AbortController());
  const histories = controllers.map(controller => client.product(query('national-focus'), controller.signal));
  const queuedFailure = assert.rejects(histories[3], { name: 'AbortError' }), activeFailure = assert.rejects(histories[0], { name: 'AbortError' });
  try {
    await tick();
    controllers[3].abort(); await queuedFailure;
    assert.equal(transport.requests.length, 3);
    const next = client.product(query('national-focus'), new AbortController().signal);
    controllers[0].abort(); await tick();
    assert.equal(transport.requests[0].signal.aborted, true);
    assert.equal(transport.requests.length, 3, 'cancellation alone does not overbook the still-unsettled HTTP body');
    transport.requests[0].finish(); await activeFailure; await tick();
    assert.equal(transport.requests.length, 4);
    const text = client.product<{ name: string }>(query('focus-artwork'), new AbortController().signal);
    const image = client.product(query('technology-artwork'), new AbortController().signal);
    const imageFailure = assert.rejects(image, /Synthetic image refusal/);
    await tick();
    transport.requests.find(request => request.body.request?.product === 'focus-artwork')!.finish({ name: 'Independent localized title' });
    transport.requests.find(request => request.body.request?.product === 'technology-artwork')!.finish({ error: 'Synthetic image refusal' }, false);
    assert.equal((await text).data.name, 'Independent localized title'); await imageFailure;
    await transport.drain();
    await Promise.all([...histories.slice(1, 3), next]);
  } finally { await transport.drain(); transport.restore(); }
});

test('source release bypasses histories and cancels only its queued and active reads', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const transport = pendingTransport();
  const released = Array.from({ length: 4 }, () => client.product(query('national-focus', 'released-source'), new AbortController().signal));
  const failures = released.map(promise => assert.rejects(promise, { name: 'AbortError' }));
  const other = client.product(query('focus-artwork', 'other-source'), new AbortController().signal);
  try {
    await tick();
    const release = client.releaseSource('released-source');
    await tick();
    const command = transport.requests.find(request => request.body.type === 'release')!;
    assert.ok(command, 'release reaches the reserved control connection');
    assert.equal(command.body.sourceId, 'released-source');
    assert.equal(transport.requests.filter(request => request.body.request?.sourceId === 'released-source').length, 3);
    assert.ok(transport.requests.filter(request => request.body.request?.sourceId === 'released-source').every(request => request.signal.aborted));
    const independent = transport.requests.find(request => request.body.request?.sourceId === 'other-source')!;
    assert.equal(independent.signal.aborted, false);
    command.finish(); await release;
    independent.finish({ title: 'Other source' }); assert.deepEqual((await other).data, { title: 'Other source' });
    await transport.drain(); await Promise.all(failures);
  } finally { await transport.drain(); transport.restore(); }
});

test('an open cancelled after its body begins releases the late lease on the control lane', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const transport = pendingTransport(), controller = new AbortController();
  const opened = client.openSource('late-open', controller.signal), failure = assert.rejects(opened, { name: 'AbortError' });
  try {
    await tick();
    controller.abort();
    transport.requests[0].finish({ id: 'late-lease', revision: 'v1', metadata: {} });
    await tick();
    const lateRelease = transport.requests.find(request => request.body.type === 'release')!;
    assert.equal(lateRelease.body.sourceId, 'late-lease');
    lateRelease.finish(); await failure;
  } finally { await transport.drain(); transport.restore(); }
});

test('the history cache request says only whether to clear', async () => {
  const bodies: unknown[] = [];
  globalThis.fetch = (async (_url: string, init: RequestInit) => {
    bodies.push(JSON.parse(String(init.body)));
    return { ok: true, text: async () => JSON.stringify({ bytes: 0, limit: 1 }) };
  }) as unknown as typeof fetch;
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  assert.deepEqual(await client.historyCache!(true, AbortSignal.timeout(1000)), { bytes: 0, limit: 1 });
  assert.deepEqual(bodies, [{ type: 'historyCache', clear: true }]);
});

test('prepared pages use reserved query capacity, preserve exact values, and reject another revision', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const transport = pendingTransport();
  const source = { id: 'prepared-source', revision: 'v1', path: 'synthetic.battleplan', metadata: {} } as import('./client').DashboardSource;
  const histories = Array.from({ length: 3 }, () => client.product(query('national-focus'), new AbortController().signal));
  try {
    await tick();
    const page = client.preparedQuery!<{ raw: string }>(source, 'focus-hour', { branchId: '1', tag: 'Synthetic country', hour: 100 }, { signal: new AbortController().signal });
    await tick();
    const http = transport.requests.find(request => request.body.type === 'preparedQuery')!;
    assert.ok(http, 'prepared-page queries do not wait for whole-history responses');
    assert.equal(http.body.refresh, false);
    http.finish({ raw: '9007199254740993' });
    assert.deepEqual(await page, { raw: '9007199254740993' });
    const stale = client.preparedQuery!(source, 'focus-hour', {}, { signal: new AbortController().signal, refresh: true });
    const refused = assert.rejects(stale, /reply does not match this recording and product/);
    await tick();
    const staleHttp = transport.requests.filter(request => request.body.type === 'preparedQuery').at(-1)!;
    assert.equal(staleHttp.body.refresh, true);
    staleHttp.body.request = { ...staleHttp.body.request!, revision: 'previous-revision' };
    staleHttp.finish({ raw: '1' });
    await refused;
    await transport.drain();
    await Promise.all(histories);
  } finally { await transport.drain(); transport.restore(); }
});

test('preparation commands echo their request identity and preserve decimal counts', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const previous = globalThis.fetch;
  const bodies: Record<string, unknown>[] = [];
  globalThis.fetch = (async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body));
    bodies.push(body);
    const reply = {
      sourceId: body.sourceId, revision: body.revision, requestId: body.id, state: 'ready',
      sections: [{ id: 'focus', label: 'National focus', state: 'ready', done: '9007199254740993', total: '9007199254740993', error: null }],
      bytes: '9007199254740993', error: null,
      backgroundEnabled: true,
    };
    if (body.id === 'stale-progress') reply.requestId = 'previous-progress';
    return { ok: true, text: async () => JSON.stringify(reply) };
  }) as unknown as typeof fetch;
  try {
    const identity = { sourceId: 'prepared-source', revision: 'v1', requestId: 'exact-progress' };
    const reply = await client.preparationStatus!(identity, new AbortController().signal);
    assert.equal(reply.bytes, '9007199254740993');
    assert.equal(reply.sections[0].done, '9007199254740993');
    assert.deepEqual(bodies[0], { type: 'preparation', id: 'exact-progress', sourceId: 'prepared-source', revision: 'v1', action: 'status' });
    await assert.rejects(client.preparationStatus!({ ...identity, requestId: 'stale-progress' }, new AbortController().signal), /does not match this recording and request/);
  } finally { globalThis.fetch = previous; }
});

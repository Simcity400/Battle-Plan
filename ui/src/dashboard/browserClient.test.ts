import { gatedHour } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { forwardRequest, type DeclaredProduct } from './request.ts';
import type { ParamsFor } from './regionTime';

const jsonBytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).buffer;

test('a page going away names itself, the owner of everything it opened or imported, so the bridge lets go of it all', async () => {
  const handlers = new Map<string, (event: { persisted: boolean }) => void>(), beacons: Blob[] = [];
  const owners: unknown[] = [];
  Object.assign(globalThis, { window: globalThis, addEventListener: (type: string, handler: (event: { persisted: boolean }) => void) => handlers.set(type, handler) });
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { sendBeacon: (_url: string, body: Blob) => beacons.push(body) } });
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
    return { ok: true, arrayBuffer: async () => jsonBytes(reply) };
  }) as unknown as typeof fetch;
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  assert.ok(!handlers.has('pagehide'), 'a page that never used the bridge (the desktop app) says nothing as it goes');
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
    let resolve!: (bytes: ArrayBuffer) => void, reject!: (error: Error) => void;
    const bytes = new Promise<ArrayBuffer>((yes, no) => { resolve = yes; reject = no; });
    const response = { ok: true, arrayBuffer: () => bytes };
    const request: PendingHttp = { body, signal: init.signal as AbortSignal, done: false,
      finish: (data = {}, ok = true) => {
        request.done = true; response.ok = ok;
        resolve(jsonBytes(body.type === 'product' && ok ? { ...body.request, data } : data));
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
async function openSources(...ids: string[]) {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const previous = globalThis.fetch;
  globalThis.fetch = (async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body));
    const branches = [{ id: '1', firstHour: 0, lastHour: 1000, gaps: [] }];
    return { ok: true, arrayBuffer: async () => jsonBytes({ id: body.path, revision: 'v1', metadata: { branches } }) };
  }) as unknown as typeof fetch;
  try { for (const id of ids) await client.openSource(id, new AbortController().signal); } finally { globalThis.fetch = previous; }
}
const FIXTURES = {
  'national-focus': { branchId: '1', country: 'GER' },
  'country-styles': { branchId: '1', hour: gatedHour(100), tags: ['GER'] },
  'technology-artwork': { tokens: ['synthetic-technology'] },
  'land-owners': {},
} as const satisfies Partial<Record<DeclaredProduct, Readonly<Record<string, unknown>>>>;
const query = (product: keyof typeof FIXTURES, sourceId = 'scheduler-source'): import('./client').ProductRequest => (forwardRequest({ id: sourceId, revision: 'v1' }, product,
  FIXTURES[product] as unknown as ParamsFor<typeof product>));

test('actual browser transport reserves interactive capacity while five histories wait and eventually returns every history', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const { INTERACTIVE_PRODUCTS } = await import('../generated/dashboard.ts');
  assert.ok(INTERACTIVE_PRODUCTS.includes('country-styles'));
  await openSources('scheduler-source');
  const transport = pendingTransport();
  try {
    const histories = Array.from({ length: 5 }, () => client.product(query('national-focus'), new AbortController().signal));
    await tick();
    assert.equal(transport.requests.length, 2, 'ordinary reads leave the interactive connections free before fetch');
    const style = client.product<{ name: string }>(query('country-styles'), new AbortController().signal);
    await tick();
    const http = transport.requests.find(request => request.body.request?.product === 'country-styles')!;
    assert.ok(http, 'dated text reaches fetch without waiting for a history reply');
    http.finish({ name: 'Localized title immediately available' });
    assert.equal((await style).data.name, 'Localized title immediately available');
    assert.ok(transport.requests.filter(request => request.body.request?.product === 'national-focus').every(request => !request.done));
    await transport.drain();
    assert.equal((await Promise.all(histories)).length, 5, 'admission queues work without omitting histories');
    assert.equal(transport.requests.filter(request => request.body.request?.product === 'national-focus').length, 5);
  } finally { await transport.drain(); transport.restore(); }
});

test('actual transport cancels queued reads without fetch, holds an aborted body slot, and preserves independent text after an image refusal', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  await openSources('scheduler-source');
  const transport = pendingTransport();
  try {
    const controllers = Array.from({ length: 4 }, () => new AbortController());
    const histories = controllers.map(controller => client.product(query('national-focus'), controller.signal));
    const queuedFailure = assert.rejects(histories[3], { name: 'AbortError' }), activeFailure = assert.rejects(histories[0], { name: 'AbortError' });
    await tick();
    controllers[3].abort(); await queuedFailure;
    assert.equal(transport.requests.length, 2);
    const next = client.product(query('national-focus'), new AbortController().signal);
    controllers[0].abort(); await tick();
    assert.equal(transport.requests[0].signal.aborted, true);
    assert.equal(transport.requests.length, 2, 'cancellation alone does not overbook the still-unsettled HTTP body');
    transport.requests[0].finish(); await activeFailure; await tick();
    assert.equal(transport.requests.length, 3);
    const text = client.product<{ name: string }>(query('country-styles'), new AbortController().signal);
    const image = client.product(query('technology-artwork'), new AbortController().signal);
    const imageFailure = assert.rejects(image, /Synthetic image refusal/);
    await tick();
    transport.requests.find(request => request.body.request?.product === 'country-styles')!.finish({ name: 'Independent localized title' });
    transport.requests.find(request => request.body.request?.product === 'technology-artwork')!.finish({ error: { kind: 'failed', message: 'Synthetic image refusal' } }, false);
    assert.equal((await text).data.name, 'Independent localized title'); await imageFailure;
    await transport.drain();
    await Promise.all([...histories.slice(1, 3), next]);
  } finally { await transport.drain(); transport.restore(); }
});

test('source release bypasses histories and cancels only its queued and active reads', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  await openSources('released-source', 'other-source');
  const transport = pendingTransport();
  try {
    const released = Array.from({ length: 4 }, () => client.product(query('national-focus', 'released-source'), new AbortController().signal));
    const failures = released.map(promise => assert.rejects(promise, { name: 'AbortError' }));
    const other = client.product(query('country-styles', 'other-source'), new AbortController().signal);
    await tick();
    const release = client.releaseSource('released-source');
    await tick();
    const command = transport.requests.find(request => request.body.type === 'release')!;
    assert.ok(command, 'release reaches the reserved control connection');
    assert.equal(command.body.sourceId, 'released-source');
    assert.equal(transport.requests.filter(request => request.body.request?.sourceId === 'released-source').length, 2);
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

test('browser product errors keep named refusals distinct from technical failures', async () => {
  const { browserDashboardClient: client } = await import('./browserClient.ts');
  const { RefusedError } = await import('./queryError.ts');
  const transport = pendingTransport();
  try {
    for (const error of [{ kind: 'refused', text: 'Synthetic data was not recorded' }, { kind: 'failed', message: 'Synthetic technical detail' }]) {
      const read = client.product(query('land-owners'), new AbortController().signal);
      const rejected = assert.rejects(read, failure => {
        assert.ok(failure instanceof Error);
        assert.equal(failure instanceof RefusedError, error.kind === 'refused');
        assert.equal(failure.message, error.kind === 'refused' ? error.text : error.message);
        return true;
      });
      await tick();
      transport.requests.at(-1)!.finish({ error }, false);
      await rejected;
    }
  } finally { transport.restore(); }
});

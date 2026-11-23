import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const script = readFileSync(new URL('../../../crates/desktop/src/ipc.js', import.meta.url), 'utf8');

type Sent = { url: string; headers: Headers; body: unknown; request: Request };
type Message = { cmd: string; callback: number; error: number; payload: unknown; options?: { headers?: Record<string, string> } };

function desktop(fetch: (sent: Sent) => Promise<Response>) {
  const sent: Sent[] = [];
  const ran: [number, unknown][] = [];
  const errors: string[] = [];
  const posted: unknown[] = [];
  const listeners = new Map<string, (event: { persisted: boolean }) => void>();
  const waiting: [number, () => void][] = [];
  const internals = {
    convertFileSrc: (path: string, protocol: string) => `http://${protocol}.localhost/${encodeURIComponent(path)}`,
    runCallback: (id: number, value: unknown) => {
      ran.push([id, value]);
      for (const [count, resolve] of waiting) if (ran.length >= count) resolve();
    },
  } as { postMessage?: (message: Message) => void; convertFileSrc: unknown; runCallback: unknown };
  const window = {
    __TAURI_INTERNALS__: internals,
    ipc: { postMessage: (message: unknown) => { posted.push(message); } },
    addEventListener: (name: string, listener: (event: { persisted: boolean }) => void) => { listeners.set(name, listener); },
  };
  const page = {
    fetch: (url: string, init: { method: string; headers: Headers; body: BodyInit }) => {
      const request = { url, headers: init.headers, body: init.body, request: new Request(url, init) };
      sent.push(request);
      return fetch(request);
    },
    console: { error: (message: string) => { errors.push(message); } },
  };
  new Function('window', 'fetch', 'console', script.replace('__INVOKE_KEY__', '"the-key"'))(window, page.fetch, page.console);
  const answered = (count: number) => new Promise<void>(resolve => { if (ran.length >= count) resolve(); else waiting.push([count, resolve]); });
  return { send: internals.postMessage!, sent, ran, errors, posted, answered, fire: (name: string, persisted = false) => listeners.get(name)?.({ persisted }) };
}

test('an invoke goes to the IPC protocol with its key and answers the callback the host names', async () => {
  const host = desktop(async () => new Response('{"rows":3}', { headers: { 'Tauri-Response': 'ok', 'Content-Type': 'application/json' } }));
  host.send({ cmd: 'replay_library', callback: 1, error: 2, payload: { folder: 'x' } });
  await host.answered(1);
  assert.equal(host.sent[0]!.url, 'http://ipc.localhost/replay_library');
  assert.equal(host.sent[0]!.headers.get('Tauri-Invoke-Key'), 'the-key');
  assert.equal(host.sent[0]!.headers.get('Tauri-Callback'), '1');
  assert.equal(host.sent[0]!.body, '{"folder":"x"}');
  assert.deepEqual(host.ran, [[1, { rows: 3 }]]);

  const refused = desktop(async () => new Response('"No such recording"', { headers: { 'Tauri-Response': 'error', 'Content-Type': 'application/json' } }));
  refused.send({ cmd: 'open_replay', callback: 1, error: 2, payload: {} });
  await refused.answered(1);
  assert.deepEqual(refused.ran, [[2, 'No such recording']]);
});

test('a byte payload is sent as its bytes', async () => {
  const host = desktop(async () => new Response(null, { headers: { 'Tauri-Response': 'ok' } }));
  host.send({ cmd: 'perf_write', callback: 1, error: 2, payload: [0, 7, 255] });
  host.send({ cmd: 'perf_write', callback: 3, error: 4, payload: new Uint8Array([9, 8]) });
  await host.answered(2);
  assert.equal(host.sent[0]!.request.headers.get('Content-Type'), 'application/octet-stream');
  assert.deepEqual([...new Uint8Array(await host.sent[0]!.request.arrayBuffer())], [0, 7, 255]);
  assert.deepEqual([...new Uint8Array(await host.sent[1]!.request.arrayBuffer())], [9, 8]);
});

test('a failed IPC request rejects that invoke with a named error, never falls back to postMessage, and the next invoke still goes through', async () => {
  let failing = true;
  const host = desktop(async () => {
    if (failing) throw new TypeError('Failed to fetch');
    return new Response('[]', { headers: { 'Tauri-Response': 'ok', 'Content-Type': 'application/json' } });
  });
  host.send({ cmd: 'content_folders', callback: 1, error: 2, payload: {} });
  await host.answered(1);
  assert.deepEqual(host.ran.map(([id]) => id), [2]);
  assert.match(String(host.ran[0]![1]), /couldn’t reach its desktop host for content_folders: .*Failed to fetch/);
  assert.equal(host.errors.length, 1);
  failing = false;
  host.send({ cmd: 'replay_library', callback: 3, error: 4, payload: {} });
  await host.answered(2);
  assert.equal(host.sent.length, 2, 'the second invoke still uses the IPC protocol');
  assert.deepEqual(host.ran[1], [3, []]);
  assert.deepEqual(host.posted, []);
});

test('requests a leaving page loses are dropped, not reported to the page or re-sent', async () => {
  let abort: (reason: unknown) => void = () => {};
  const host = desktop(() => new Promise<Response>((_, reject) => { abort = reject; }));
  host.send({ cmd: 'replay_library', callback: 1, error: 2, payload: {} });
  host.fire('pagehide');
  abort(new TypeError('Failed to fetch'));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(host.sent.length, 1, 'nothing is re-sent');
  assert.deepEqual(host.ran, []);
  assert.deepEqual(host.posted, []);
  assert.deepEqual(host.errors, []);
});

test('a page kept in the back/forward cache still hears a request that fails, and a page shown again is live', async () => {
  const aborts: ((reason: unknown) => void)[] = [];
  const host = desktop(() => new Promise<Response>((_, reject) => { aborts.push(reject); }));
  host.send({ cmd: 'replay_library', callback: 1, error: 2, payload: {} });
  host.fire('pagehide', true);
  aborts[0]!(new TypeError('Failed to fetch'));
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(host.ran.map(([id]) => id), [2], 'the cached page’s invoke rejects and frees its slot');

  host.send({ cmd: 'content_folders', callback: 3, error: 4, payload: {} });
  host.fire('pagehide');
  host.fire('pageshow', true);
  aborts[1]!(new TypeError('Failed to fetch'));
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(host.ran.map(([id]) => id), [2, 4], 'a page restored after leaving reports its failures again');
  assert.deepEqual(host.posted, []);
});

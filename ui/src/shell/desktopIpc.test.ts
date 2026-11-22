import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const script = readFileSync(new URL('../../../crates/desktop/src/ipc.js', import.meta.url), 'utf8');

type Sent = { url: string; headers: Headers; body: unknown };
type Message = { cmd: string; callback: number; error: number; payload: unknown; options?: { headers?: Record<string, string> } };

function desktop(fetch: (sent: Sent) => Promise<Response>) {
  const sent: Sent[] = [];
  const ran: [number, unknown][] = [];
  const errors: string[] = [];
  const posted: unknown[] = [];
  const listeners = new Map<string, () => void>();
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
    addEventListener: (name: string, listener: () => void) => { listeners.set(name, listener); },
  };
  const page = {
    fetch: (url: string, init: { headers: Headers; body: unknown }) => {
      const request = { url, headers: init.headers, body: init.body };
      sent.push(request);
      return fetch(request);
    },
    console: { error: (message: string) => { errors.push(message); } },
  };
  new Function('window', 'fetch', 'console', script.replace('__INVOKE_KEY__', '"the-key"'))(window, page.fetch, page.console);
  const answered = (count: number) => new Promise<void>(resolve => { if (ran.length >= count) resolve(); else waiting.push([count, resolve]); });
  return { send: internals.postMessage!, sent, ran, errors, posted, answered, fire: (name: string) => listeners.get(name)?.() };
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

test('a failed IPC request rejects that invoke with a named error and never falls back to postMessage', async () => {
  const host = desktop(async () => { throw new TypeError('Failed to fetch'); });
  host.send({ cmd: 'content_folders', callback: 1, error: 2, payload: {} });
  host.send({ cmd: 'replay_library', callback: 3, error: 4, payload: {} });
  await host.answered(2);
  assert.equal(host.sent.length, 2, 'the second invoke still uses the IPC protocol');
  assert.deepEqual(host.posted, []);
  assert.deepEqual(host.ran.map(([id]) => id), [2, 4]);
  assert.match(String(host.ran[0]![1]), /couldn’t reach its desktop host for content_folders: .*Failed to fetch/);
  assert.equal(host.errors.length, 2);
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

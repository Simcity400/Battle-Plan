/** Replies at least this long are parsed in a worker; shorter ones cost less than the trip. */
const OFF_THREAD_BYTES = 64 * 1024;

let worker: Worker | null = null;
let next = 0;
const waiting = new Map<number, { resolve: (value: unknown) => void; reject: (error: unknown) => void }>();

function parser(): Worker {
  if (worker) return worker;
  const started = new Worker(new URL('./replyParser.worker.ts', import.meta.url), { type: 'module' });
  started.onmessage = (event: MessageEvent<{ id: number; value?: unknown; error?: string }>) => {
    const reply = waiting.get(event.data.id);
    if (!reply) return;
    waiting.delete(event.data.id);
    if (event.data.error !== undefined) reply.reject(new SyntaxError(event.data.error));
    else reply.resolve(event.data.value);
  };
  // A worker that fails fails what it holds; the next large reply starts another.
  started.onerror = event => {
    event.preventDefault();
    started.terminate();
    if (worker === started) worker = null;
    const failure = new Error(`Reply parsing failed: ${event.message}`);
    for (const reply of waiting.values()) reply.reject(failure);
    waiting.clear();
  };
  return worker = started;
}

/** `bytes` decoded and parsed as JSON: a large reply in a worker, which takes the bytes. */
export function parseReplyBytes(bytes: ArrayBuffer | Uint8Array): Promise<unknown> {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  // Outside a browser (the unit tests) there are no workers.
  if (view.byteLength < OFF_THREAD_BYTES || typeof Worker === 'undefined') return Promise.resolve(JSON.parse(new TextDecoder().decode(view)) as unknown);
  const off = parser();
  // Only a whole buffer can be handed over.
  const owned = (view.byteOffset === 0 && view.byteLength === view.buffer.byteLength && view.buffer instanceof ArrayBuffer
    ? view.buffer : view.slice().buffer) as ArrayBuffer;
  const id = next++;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    off.postMessage({ id, bytes: owned }, [owned]);
  });
}

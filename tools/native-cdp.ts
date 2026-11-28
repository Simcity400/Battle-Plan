import { WebSocket } from 'undici';

type CdpMessage = {
  id?: number;
  method?: string;
  params?: any;
  result?: any;
  error?: unknown;
};

export class NativeCdp {
  readonly #socket: WebSocket;
  readonly #disconnected = new AbortController();
  readonly #opened: Promise<void>;
  readonly #closed: Promise<void>;
  readonly #pending = new Map<number, (message: CdpMessage) => void>();
  readonly #listeners = new Set<(message: CdpMessage) => void>();
  #sequence = 0;

  constructor(endpoint: string) {
    const socket = this.#socket = new WebSocket(endpoint);
    let opened!: () => void;
    let closed!: () => void;
    this.#opened = new Promise<void>(resolve => { opened = resolve; });
    this.#closed = new Promise<void>(resolve => { closed = resolve; });
    const disconnected = (message: string) => {
      this.#disconnected.abort(new Error(message));
      opened();
    };
    socket.addEventListener('open', () => opened(), { once: true });
    socket.addEventListener('error', () => disconnected('CDP connection failed'), { once: true });
    socket.addEventListener('close', () => {
      disconnected('CDP connection closed');
      closed();
    }, { once: true });
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data)) as CdpMessage;
      if (message.id === undefined) {
        for (const listener of this.#listeners) listener(message);
      } else {
        this.#pending.get(message.id)?.(message);
      }
    });
  }

  get disconnected(): AbortSignal { return this.#disconnected.signal; }

  onEvent(listener: (message: CdpMessage) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  async send(method: string, params: Record<string, unknown> = {}, signal = AbortSignal.timeout(30_000)): Promise<any> {
    const until = AbortSignal.any([signal, this.disconnected]);
    until.throwIfAborted();
    const id = ++this.#sequence;
    let reject!: (reason: unknown) => void;
    const abort = () => reject(until.reason);
    const promise = new Promise<any>((resolve, fail) => {
      reject = fail;
      this.#pending.set(id, message => {
        if (message.error) fail(new Error(`${method}: ${JSON.stringify(message.error)}`));
        else resolve(message.result);
      });
    });
    until.addEventListener('abort', abort, { once: true });
    void this.#opened.then(() => {
      if (until.aborted) return;
      try { this.#socket.send(JSON.stringify({ id, method, params })); }
      catch (error) { reject(error); }
    });
    try {
      return await promise;
    } finally {
      this.#pending.delete(id);
      until.removeEventListener('abort', abort);
    }
  }

  async close(): Promise<void> {
    this.#socket.close();
    await this.#closed;
  }
}

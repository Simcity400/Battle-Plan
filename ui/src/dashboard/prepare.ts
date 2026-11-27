import type { DashboardClient, ProductReply, ProductRequest } from './client';
import { busyCount, inBackground } from '../shell/busy.ts';


const requestKey = (request: ProductRequest) => JSON.stringify([request.sourceId, request.revision, request.product, request.params]);

interface Waiting { key: string | null; ready: () => boolean; send: (background: boolean) => void }

export interface PrepareFrames { request(run: () => void): number; cancel(id: number): void }
const animationFrames: PrepareFrames = typeof requestAnimationFrame === 'function'
  ? { request: run => requestAnimationFrame(run), cancel: id => cancelAnimationFrame(id) }
  : { request: run => setTimeout(run, 16) as unknown as number, cancel: id => clearTimeout(id) };

export class PrepareGate {
  private waiting: Waiting[] = [];
  private running = 0;
  private readonly sent = new Set<string>();
  private frame = 0;
  private asked: ProductRequest[] = [];
  private listeners = new Set<() => void>();
  private readonly idle: () => boolean;
  private readonly frames: PrepareFrames;

  constructor(idle: () => boolean = () => busyCount() === 0, frames: PrepareFrames = animationFrames) {
    this.idle = idle;
    this.frames = frames;
  }

  ask<T>(request: ProductRequest, send: (signal: AbortSignal, background: boolean) => Promise<T>, signal: AbortSignal, ready = () => true): Promise<T> {
    return this.enqueue(requestKey(request), send, signal, ready);
  }

  wait(signal: AbortSignal, ready: () => boolean): Promise<void> {
    return this.enqueue(null, async () => {}, signal, ready);
  }

  private enqueue<T>(key: string | null, send: (signal: AbortSignal, background: boolean) => Promise<T>, signal: AbortSignal, ready: () => boolean): Promise<T> {
    if (signal.aborted) return Promise.reject(new DOMException('Request cancelled', 'AbortError'));
    return new Promise<T>((resolve, reject) => {
      const entry: Waiting = {
        key, ready,
        send: background => {
          signal.removeEventListener('abort', leave);
          if (background) { this.running++; if (key !== null) this.sent.add(key); }
          send(signal, background).then(resolve, reject).finally(() => {
            if (!background) return;
            this.running--;
            if (key !== null) this.sent.delete(key);
            this.pump();
          });
        },
      };
      const leave = () => {
        const at = this.waiting.indexOf(entry);
        if (at >= 0) this.waiting.splice(at, 1);
        reject(new DOMException('Request cancelled', 'AbortError'));
        this.told();
      };
      signal.addEventListener('abort', leave, { once: true });
      this.waiting.push(entry);
      this.pump();
    });
  }

  hold(request: ProductRequest, signal: AbortSignal): AbortSignal {
    const key = requestKey(request), held = new AbortController();
    const follow = () => { if (!this.sent.has(key)) held.abort(); };
    if (signal.aborted) follow(); else signal.addEventListener('abort', follow, { once: true });
    return held.signal;
  }

  promote(request: ProductRequest): void {
    const key = requestKey(request);
    const at = this.waiting.findIndex(entry => entry.key === key && entry.ready());
    if (at < 0) return;
    const [entry] = this.waiting.splice(at, 1);
    entry.send(false);
    this.told();
  }

  get pending(): number { return this.waiting.length + this.running; }

  note(request: ProductRequest): void { this.asked.push(request); }

  drain(): ProductRequest[] { return this.asked.splice(0); }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  private told(): void { for (const listener of [...this.listeners]) listener(); }

  private pump(): void {
    if (this.frame) { this.frames.cancel(this.frame); this.frame = 0; }
    if (this.running === 0 && this.waiting.length) {
      const at = this.idle() ? this.waiting.findIndex(entry => entry.ready()) : -1;
      if (at >= 0) this.waiting.splice(at, 1)[0]!.send(true);
      else this.frame = this.frames.request(() => { this.frame = 0; this.pump(); });
    }
    this.told();
  }
}

interface Prepared { base?: DashboardClient; gate?: PrepareGate }

export function baseClient(client: DashboardClient): DashboardClient {
  return (client as Prepared).base ?? client;
}

const gates = new WeakMap<DashboardClient, PrepareGate>();
export function prepareGate(client: DashboardClient): PrepareGate {
  const base = baseClient(client);
  let gate = gates.get(base);
  if (!gate) gates.set(base, gate = new PrepareGate());
  return gate;
}

export function preparingClient(base: DashboardClient): DashboardClient {
  const gate = prepareGate(base);
  const client = Object.create(base) as DashboardClient & Prepared;
  client.base = base;
  client.gate = gate;
  client.product = <T,>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>> =>
    gate.ask(request, (sent, background) => background ? inBackground(() => base.product<T>(request, sent)) : base.product<T>(request, sent), signal);
  return client;
}

export const preparing = (client: DashboardClient): boolean => (client as Prepared).gate !== undefined;

export function noteRead(client: DashboardClient, request: ProductRequest): void {
  (client as Prepared).gate?.note(request);
}

export function askedByPage(client: DashboardClient, request: ProductRequest): void {
  if (!preparing(client)) gates.get(baseClient(client))?.promote(request);
}

export class PerClient<V> {
  private held = new WeakMap<DashboardClient, V>();
  get(client: DashboardClient): V | undefined { return this.held.get(baseClient(client)); }
  set(client: DashboardClient, value: V): this { this.held.set(baseClient(client), value); return this; }
  has(client: DashboardClient): boolean { return this.held.has(baseClient(client)); }
  delete(client: DashboardClient): boolean { return this.held.delete(baseClient(client)); }
}

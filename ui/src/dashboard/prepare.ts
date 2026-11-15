import type { DashboardClient, ProductReply, ProductRequest } from './client';
import { busyCount, inBackground } from '../shell/busy.ts';

/** The prepare lane: what the page does not show (its rows below the fold, every other page) is asked through it once the
 * page has settled, so a scroll or a page switch finds its replies held. It holds one request in flight at a time, and
 * sends none while the page has anything of its own in flight. Every lane of the service has at least two threads, so a
 * prepared request holds at most one of any lane's and a request the page asks finds another. */

const requestKey = (request: ProductRequest) => JSON.stringify([request.sourceId, request.revision, request.product, request.params]);

interface Waiting { key: string; request: ProductRequest; send: (background: boolean) => void }

export interface PrepareFrames { request(run: () => void): number; cancel(id: number): void }
const animationFrames: PrepareFrames = typeof requestAnimationFrame === 'function'
  ? { request: run => requestAnimationFrame(run), cancel: id => cancelAnimationFrame(id) }
  : { request: run => setTimeout(run, 16) as unknown as number, cancel: id => clearTimeout(id) };

export class PrepareGate {
  private waiting: Waiting[] = [];
  private running = 0;
  private frame = 0;
  private asked: ProductRequest[] = [];
  private listeners = new Set<() => void>();
  private readonly idle: () => boolean;
  private readonly frames: PrepareFrames;

  /** `idle`: whether the page has nothing of its own in flight; asked each frame while a request waits. */
  constructor(idle: () => boolean = () => busyCount() === 0, frames: PrepareFrames = animationFrames) {
    this.idle = idle;
    this.frames = frames;
  }

  /** `send`s `request` in its turn: once nothing else of the lane is in flight and the page is idle. */
  ask<T>(request: ProductRequest, send: (signal: AbortSignal, background: boolean) => Promise<T>, signal: AbortSignal): Promise<T> {
    if (signal.aborted) return Promise.reject(new DOMException('Request cancelled', 'AbortError'));
    return new Promise<T>((resolve, reject) => {
      const entry: Waiting = {
        key: requestKey(request), request,
        send: background => {
          signal.removeEventListener('abort', leave);
          if (background) this.running++;
          send(signal, background).then(resolve, reject).finally(() => {
            if (!background) return;
            this.running--;
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

  /** A request the page itself now waits on goes at once, out of the lane. */
  promote(request: ProductRequest): void {
    const key = requestKey(request);
    const at = this.waiting.findIndex(entry => entry.key === key);
    if (at < 0) return;
    const [entry] = this.waiting.splice(at, 1);
    entry.send(false);
    this.told();
  }

  /** Every request still waiting goes at once, out of the lane: called once the lane's own askers have left, so what
   * waits is wanted by the page. */
  release(): void {
    for (const entry of this.waiting.splice(0)) entry.send(false);
    this.told();
  }

  /** Requests waiting or in flight. */
  get pending(): number { return this.waiting.length + this.running; }

  /** Notes a product the lane's widgets read, asked or held. */
  note(request: ProductRequest): void { this.asked.push(request); }

  /** The products noted since the last call. */
  drain(): ProductRequest[] { return this.asked.splice(0); }

  /** Hears each change of `pending`. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  private told(): void { for (const listener of [...this.listeners]) listener(); }

  private pump(): void {
    if (this.frame) { this.frames.cancel(this.frame); this.frame = 0; }
    if (this.running === 0 && this.waiting.length) {
      if (this.idle()) this.waiting.shift()!.send(true);
      else this.frame = this.frames.request(() => { this.frame = 0; this.pump(); });
    }
    this.told();
  }
}

interface Prepared { base?: DashboardClient; gate?: PrepareGate }

/** The client `client` asks through: a prepared client's base. Everything kept per client is kept per base, so what the
 * lane reads is held for the page. */
export function baseClient(client: DashboardClient): DashboardClient {
  return (client as Prepared).base ?? client;
}

const gates = new WeakMap<DashboardClient, PrepareGate>();
/** The prepare lane of `client`'s base. */
export function prepareGate(client: DashboardClient): PrepareGate {
  const base = baseClient(client);
  let gate = gates.get(base);
  if (!gate) gates.set(base, gate = new PrepareGate());
  return gate;
}

/** `base`, its product requests asked through its prepare lane. */
export function preparingClient(base: DashboardClient): DashboardClient {
  const gate = prepareGate(base);
  const client = Object.create(base) as DashboardClient & Prepared;
  client.base = base;
  client.gate = gate;
  client.product = <T,>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>> =>
    gate.ask(request, (sent, background) => background ? inBackground(() => base.product<T>(request, sent)) : base.product<T>(request, sent), signal);
  return client;
}

/** Whether `client` asks through the prepare lane. */
export const preparing = (client: DashboardClient): boolean => (client as Prepared).gate !== undefined;

/** `client` reads `request`: noted when it is the lane's. */
export function noteRead(client: DashboardClient, request: ProductRequest): void {
  (client as Prepared).gate?.note(request);
}

/** The page asks `request` through `client` and joins a load the lane may hold waiting: it goes at once. */
export function askedByPage(client: DashboardClient, request: ProductRequest): void {
  if (!preparing(client)) gates.get(baseClient(client))?.promote(request);
}

/** A map kept per client, each prepared client standing for its base. */
export class PerClient<V> {
  private held = new WeakMap<DashboardClient, V>();
  get(client: DashboardClient): V | undefined { return this.held.get(baseClient(client)); }
  set(client: DashboardClient, value: V): this { this.held.set(baseClient(client), value); return this; }
  has(client: DashboardClient): boolean { return this.held.has(baseClient(client)); }
  delete(client: DashboardClient): boolean { return this.held.delete(baseClient(client)); }
}

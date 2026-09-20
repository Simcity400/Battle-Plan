import { errorMessage } from '../errors.ts';
import type { DashboardClient, DashboardQuery, DashboardResult } from './client';
import { mergeSeriesChunks } from './model.ts';

export interface QuerySnapshot {
  status: 'idle' | 'loading' | 'ready' | 'error';
  result: DashboardResult | null;
  error: string | null;
}
interface Entry {
  key: string;
  request: DashboardQuery;
  snapshot: QuerySnapshot;
  listeners: Set<() => void>;
  consumers: number;
  prefetchers: number;
  version: number;
  epoch: number;
  controller: AbortController | null;
  speculative: boolean;
  queuedAt: number;
  touched: number;
  bytes: number;
}

export function queryKey(query: DashboardQuery): string {
  return JSON.stringify([query.sourceId, query.revision, query.branchId, [...new Set(query.countryIds)].sort(),
    query.firstHour, query.lastHour, query.kind, query.maxPoints]);
}

/** Shared demand ownership. Removing one widget never cancels a sibling's lease. */
export class QueryStore {
  private entries = new Map<string, Entry>();
  private pending = new Set<Entry>();
  private foregroundActive = false;
  private backgroundActive = false;
  private order = 0;
  private usageListeners = new Set<() => void>();
  private client: DashboardClient;
  private residentBytes: number;
  private cacheBytes: number;
  constructor(client: DashboardClient, residentBytes = 64 * 1024 * 1024, cacheBytes = 16 * 1024 * 1024) {
    this.client = client;
    this.residentBytes = residentBytes;
    this.cacheBytes = cacheBytes;
  }

  subscribeUsage = (listener: () => void) => { this.usageListeners.add(listener); return () => { this.usageListeners.delete(listener); }; };
  getUsage = () => [...this.entries.values()].reduce((bytes, entry) => bytes + entry.bytes, 0);
  getLimit = () => this.residentBytes;
  private usageChanged(): void { for (const listener of this.usageListeners) listener(); }

  entry(request: DashboardQuery): Entry {
    const key = queryKey(request);
    const found = this.entries.get(key);
    if (found) return found;
    const entry: Entry = { key, request: { ...request, countryIds: [...new Set(request.countryIds)].sort() },
      snapshot: { status: 'idle', result: null, error: null }, listeners: new Set(), consumers: 0, prefetchers: 0,
      version: 0, epoch: 0, controller: null, speculative: false, queuedAt: 0, touched: ++this.order, bytes: 0 };
    this.entries.set(key, entry);
    return entry;
  }

  retain(entry: Entry, listener: (() => void) | null): () => void {
    // StrictMode may retain the same object after a cleanup microtask.
    this.entries.set(entry.key, entry);
    if (listener) { entry.listeners.add(listener); entry.consumers += 1; } else entry.prefetchers += 1;
    entry.touched = ++this.order;
    if (listener && entry.controller && entry.speculative) this.retry(entry);
    if (entry.snapshot.status === 'idle') this.enqueue(entry);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (listener) { entry.listeners.delete(listener); entry.consumers -= 1; } else entry.prefetchers -= 1;
      queueMicrotask(() => {
        if (entry.consumers || entry.prefetchers) return;
        this.pending.delete(entry);
        if (entry.controller) {
          entry.epoch += 1;
          entry.controller.abort();
          entry.controller = null;
        }
        if (entry.snapshot.status === 'loading') this.publish(entry, { status: 'idle', result: null, error: null });
        this.prune();
      });
    };
  }

  retry(entry: Entry): void {
    entry.epoch += 1;
    entry.controller?.abort();
    entry.controller = null;
    entry.bytes = 0;
    this.publish(entry, { status: 'idle', result: null, error: null });
    this.usageChanged();
    if (entry.consumers || entry.prefetchers) this.enqueue(entry);
  }

  clear(): void {
    this.pending.clear();
    for (const entry of this.entries.values()) {
      entry.epoch += 1;
      entry.controller?.abort();
      entry.controller = null;
      entry.bytes = 0;
      this.publish(entry, { status: 'idle', result: null, error: null });
    }
    this.entries.clear();
    this.usageChanged();
  }

  private publish(entry: Entry, snapshot: QuerySnapshot): void {
    entry.snapshot = snapshot;
    entry.version += 1;
    for (const listener of entry.listeners) listener();
  }

  private enqueue(entry: Entry): void {
    if (entry.controller || this.pending.has(entry)) return;
    entry.queuedAt = ++this.order;
    this.pending.add(entry);
    this.publish(entry, { status: 'loading', result: null, error: null });
    // Batch independent subscriptions from the same React commit.
    queueMicrotask(() => this.drain());
  }

  private drain(): void {
    while (this.pending.size) {
      const entries = [...this.pending].filter((entry) => entry.consumers || entry.prefetchers);
      if (!entries.length) { this.pending.clear(); return; }
      const priority = (entry: Entry) => entry.consumers ? entry.request.kind === 'exact' ? 0 : 1 : 2;
      entries.sort((a, b) => priority(a) - priority(b) || a.queuedAt - b.queuedAt);
      // Reserve independent lanes: long histories cannot occupy every slot and
      // block a seek, and a stream of current-hour requests cannot starve history.
      const entry = entries.find((candidate) => priority(candidate) === 0 ? !this.foregroundActive : !this.backgroundActive);
      if (!entry) return;
      this.pending.delete(entry);
      void this.run(entry, priority(entry) === 0);
    }
  }

  private async run(entry: Entry, foreground: boolean): Promise<void> {
    const controller = new AbortController();
    const epoch = ++entry.epoch;
    entry.controller = controller;
    entry.speculative = entry.consumers === 0;
    if (foreground) this.foregroundActive = true;
    else this.backgroundActive = true;
    try {
      const priority = entry.consumers ? entry.request.kind === 'exact' ? 'current' : 'history' : 'prefetch';
      const received = await this.client.query(entry.request, controller.signal, priority);
      if (controller.signal.aborted || entry.epoch !== epoch) return;
      const result = { ...received, series: mergeSeriesChunks(received.series) };
      if (result.sourceId !== entry.request.sourceId || result.revision !== entry.request.revision
        || result.series.length !== entry.request.countryIds.length
        || new Set(result.series.map((series) => series.countryId)).size !== result.series.length
        || result.series.some((series) => series.revision !== entry.request.revision || series.branchId !== entry.request.branchId
          || series.firstHour !== entry.request.firstHour || series.lastHour !== entry.request.lastHour
          || (entry.request.kind === 'exact' && (series.downsampled || series.points.some((point) => point.firstHour !== point.lastHour)))
          || !entry.request.countryIds.includes(series.countryId))) throw new Error('The recording or query identity changed. Reopen the source to refresh this view.');
      // This is an explicit admission estimate, not a claim about V8 heap usage.
      const points = result.series.reduce((count, series) => count + series.points.length, 0);
      const estimatedBytes = JSON.stringify(result).length * 2 + points * 320 + result.series.length * 512;
      this.admit(entry, estimatedBytes);
      entry.bytes = estimatedBytes;
      entry.touched = ++this.order;
      this.publish(entry, { status: 'ready', result, error: null });
      this.usageChanged();
    } catch (cause) {
      if (!controller.signal.aborted && entry.epoch === epoch) this.publish(entry, entry.consumers
        ? { status: 'error', result: null, error: errorMessage(cause) }
        : { status: 'idle', result: null, error: null });
    } finally {
      if (foreground) this.foregroundActive = false;
      else this.backgroundActive = false;
      if (entry.controller === controller) entry.controller = null;
      this.prune();
      this.drain();
    }
  }

  private admit(incoming: Entry, bytes: number): void {
    let resident = this.getUsage() - incoming.bytes;
    const idle = [...this.entries.values()].filter((entry) => entry !== incoming && !entry.consumers && !entry.controller && entry.bytes > 0)
      .sort((a, b) => a.touched - b.touched);
    for (const entry of idle) {
      if (resident + bytes <= this.residentBytes) break;
      resident -= entry.bytes;
      entry.bytes = 0;
      this.publish(entry, { status: 'idle', result: null, error: null });
      if (!entry.prefetchers) this.entries.delete(entry.key);
    }
    this.usageChanged();
    if (resident + bytes > this.residentBytes) throw new Error(`This series exceeds the dashboard’s ${Math.round(this.residentBytes / 1024 / 1024)} MiB estimated result allowance. Remove or narrow another view, then retry. Ready views are retained.`);
  }

  private prune(): void {
    // Active consumers own their current result. Only idle reusable output is cached.
    const idle = [...this.entries.values()].filter((entry) => !entry.consumers && !entry.prefetchers && !entry.controller)
      .sort((a, b) => a.touched - b.touched);
    let bytes = idle.reduce((sum, entry) => sum + entry.bytes, 0);
    let count = idle.length;
    for (const entry of idle) {
      if (bytes <= this.cacheBytes && count <= 64 && entry.snapshot.status === 'ready') continue;
      bytes -= entry.bytes;
      count -= 1;
      this.entries.delete(entry.key);
    }
    this.usageChanged();
  }
}

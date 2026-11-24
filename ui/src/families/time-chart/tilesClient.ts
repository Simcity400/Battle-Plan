import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { validateProductReply } from '../../dashboard/client.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundWindow } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { TileReply } from '../../dashboard/tiles/tile.ts';
import type { HourSpan } from '../../generated/dashboard';
import type { TimeChartClosedTransfer } from './laneView.ts';
import {
  TILE_BINS,
  decodeTilesStrict,
  seriesKey,
  tileIdentityKey,
  type DecodedTile,
  type TileRequestIdentity,
} from './tiles.ts';

const MAX_CACHE_BYTES = 24 * 1024 * 1024;
const COUNTRY_BATCH = 64;

export interface ClosedTilesRequest {
  readonly source: Pick<DashboardSource, 'id' | 'revision'>;
  readonly branchId: string;
  readonly kind: string;
  readonly measure: string;
  readonly role: string;
  readonly tags: readonly string[];
  readonly binHours: number;
  readonly firstTile: number;
  readonly lastTile: number;
  readonly lastHour: number;
  readonly withMarks: boolean;
  readonly within?: ReadonlyMap<string, readonly HourSpan[]>;
  readonly span: (firstHour: number, lastHour: number) => BoundWindow | null;
}

function normalizedWithin(
  tags: readonly string[],
  within: ReadonlyMap<string, readonly HourSpan[]> | undefined,
  span: (firstHour: number, lastHour: number) => BoundWindow | null,
): Map<string, readonly BoundWindow[]> | undefined {
  if (!within) return undefined;
  const out = new Map<string, readonly BoundWindow[]>();
  for (const tag of tags) {
    const raw = within.get(tag) ?? [];
    const clipped: BoundWindow[] = [];
    for (const item of raw) {
      const bounded = span(item.firstHour, item.lastHour);
      if (bounded) clipped.push(bounded);
    }
    out.set(tag, clipped);
  }
  return out;
}

function tileCovers(tile: DecodedTile, lastHour: number): boolean {
  if (tile.immutable) return true;
  if (tile.filled === null) return false;
  return tile.filled >= Math.min(tile.lastHour, lastHour);
}

export function unrecordedFromTransfers(transfers: readonly TimeChartClosedTransfer[]): string | null {
  if (!transfers.length) return null;
  let reason: string | null = null;
  for (const transfer of transfers) {
    if (!transfer.tiles.length) return null;
    for (const tile of transfer.tiles) {
      for (let i = 0; i < tile.present.length; i++) {
        if (tile.present[i] !== 0) return null;
      }
      if (!tile.missing.length) return null;
      for (const span of tile.missing) {
        if (reason === null) reason = span.reason;
        else if (reason !== span.reason) return null;
      }
    }
  }
  return reason;
}

export class ClosedTilesCache {
  private readonly entries = new Map<string, DecodedTile>();
  private readonly inflight = new Map<string, Promise<DecodedTile>>();
  private totalBytes = 0;

  get(key: string): DecodedTile | undefined {
    const found = this.entries.get(key);
    if (!found) return undefined;
    this.entries.delete(key);
    this.entries.set(key, found);
    return found;
  }

  set(tile: DecodedTile): void {
    const existing = this.entries.get(tile.key);
    if (existing) {
      this.totalBytes -= existing.bytes;
      this.entries.delete(tile.key);
    }
    this.entries.set(tile.key, tile);
    this.totalBytes += tile.bytes;
    while (this.totalBytes > MAX_CACHE_BYTES && this.entries.size > 1) {
      const oldestKey = this.entries.keys().next().value;
      if (oldestKey === undefined) break;
      const oldest = this.entries.get(oldestKey)!;
      this.entries.delete(oldestKey);
      this.totalBytes -= oldest.bytes;
    }
  }

  collect(req: ClosedTilesRequest): {
    readonly transfers: TimeChartClosedTransfer[];
    readonly missing: Map<number, TileRequestIdentity[]>;
    readonly signature: string;
  } {
    const clippedWithin = normalizedWithin(req.tags, req.within, req.span);
    const transfers: TimeChartClosedTransfer[] = [];
    const missing = new Map<number, TileRequestIdentity[]>();
    const sigParts: string[] = [];

    for (const tag of req.tags) {
      const tiles: DecodedTile[] = [];
      const tagWithin = clippedWithin?.get(tag);
      for (let index = req.firstTile; index <= req.lastTile; index++) {
        const identity: TileRequestIdentity = {
          sourceId: req.source.id,
          revision: req.source.revision,
          branchId: req.branchId,
          kind: req.kind,
          measure: req.measure,
          role: req.role,
          country: tag,
          binHours: req.binHours,
          index,
          withMarks: req.withMarks,
          ...(tagWithin ? { within: tagWithin } : {}),
        };
        const key = tileIdentityKey(identity);
        const cached = this.get(key);
        if (cached && tileCovers(cached, req.lastHour)) {
          tiles.push(cached);
          sigParts.push(`${key}@${cached.filled ?? 'n'}`);
        } else {
          if (cached) {
            tiles.push(cached);
            sigParts.push(`${key}@${cached.filled ?? 'n'}`);
          }
          let list = missing.get(index);
          if (!list) missing.set(index, list = []);
          list.push(identity);
        }
      }
      if (tiles.length > 0) {
        transfers.push({
          key: seriesKey(tag, req.role),
          binHours: req.binHours,
          firstIndex: req.firstTile,
          lastIndex: req.lastTile,
          tiles,
        });
      }
    }

    return { transfers, missing, signature: `${req.binHours}|${sigParts.join(';')}` };
  }

  async fetchMissing(
    client: DashboardClient,
    req: ClosedTilesRequest,
    missing: ReadonlyMap<number, readonly TileRequestIdentity[]>,
    signal: AbortSignal,
  ): Promise<boolean> {
    if (missing.size === 0) return false;
    const tasks: Promise<void>[] = [];
    let updated = false;

    for (const [index, identities] of missing) {
      const pending: TileRequestIdentity[] = [];
      for (const id of identities) {
        const key = tileIdentityKey(id);
        const active = this.inflight.get(key);
        if (active) {
          tasks.push(active.then(() => { updated = true; }));
        } else {
          pending.push(id);
        }
      }
      for (let start = 0; start < pending.length; start += COUNTRY_BATCH) {
        const batch = pending.slice(start, start + COUNTRY_BATCH);
        const clippedWithin = normalizedWithin(batch.map(id => id.country), req.within, req.span);
        const hasWithin = clippedWithin !== undefined;
        const request = productRequest({
          sourceId: req.source.id,
          revision: req.source.revision,
          product: 'series-tiles',
          params: {
            branchId: req.branchId,
            family: { kind: req.kind, measure: req.measure },
            binHours: req.binHours,
            index,
            countries: batch.map(id => countryFromKey(id.country)),
            ...(hasWithin ? { within: batch.map(id => [countryFromKey(id.country), clippedWithin.get(id.country) ?? []] as const) } : {}),
            ...(req.withMarks ? { withMarks: true } : {}),
          },
        });
        const resolvers = new Map<string, { resolve: (t: DecodedTile) => void; reject: (e: unknown) => void }>();
        for (const id of batch) {
          const key = tileIdentityKey(id);
          const promise = new Promise<DecodedTile>((resolve, reject) => {
            resolvers.set(key, { resolve, reject });
          });
          this.inflight.set(key, promise);
        }
        const batchPromise = client.product<TileReply>(request, signal).then(reply => {
          const validated = validateProductReply(reply, request).data;
          const decoded = decodeTilesStrict(validated, batch);
          for (const tile of decoded) {
            this.set(tile);
            this.inflight.delete(tile.key);
            resolvers.get(tile.key)?.resolve(tile);
          }
          updated = true;
        }).catch(error => {
          for (const id of batch) {
            const key = tileIdentityKey(id);
            this.inflight.delete(key);
            resolvers.get(key)?.reject(error);
          }
          throw error;
        });
        tasks.push(batchPromise);
      }
    }

    await Promise.all(tasks);
    return updated;
  }
}

const cacheByClient = new WeakMap<DashboardClient, ClosedTilesCache>();

export function closedTilesCache(client: DashboardClient): ClosedTilesCache {
  let cache = cacheByClient.get(client);
  if (!cache) {
    cache = new ClosedTilesCache();
    cacheByClient.set(client, cache);
  }
  return cache;
}

export async function fetchExactClosedTiles(
  client: DashboardClient,
  req: ClosedTilesRequest,
  signal: AbortSignal,
): Promise<TimeChartClosedTransfer[]> {
  const cache = closedTilesCache(client);
  const first = cache.collect(req);
  if (first.missing.size > 0) {
    await cache.fetchMissing(client, req, first.missing, signal);
  }
  return cache.collect(req).transfers;
}

export function initialTileBounds(from: number, to: number, lastHour: number, binHours: number, aheadBins: number): {
  readonly firstTile: number;
  readonly lastTile: number;
} {
  const tileSpan = TILE_BINS * binHours;
  const firstTile = Math.floor(from / tileSpan);
  const ahead = Math.min(lastHour, to + aheadBins * binHours);
  const lastTile = Math.max(firstTile, Math.floor(Math.max(to, ahead) / tileSpan));
  return { firstTile, lastTile };
}

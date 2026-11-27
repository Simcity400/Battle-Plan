import type { Tile } from './tile.ts';
import type { ProductReply, ProductRequest } from '../client.ts';

export const TILE_CACHE_BYTES = 64 * 1024 * 1024;

interface CachedLane {
  readonly config: string;
  readonly id: number;
  uses: number;
}

interface CachedTile {
  readonly tile: Tile | StripValue;
  readonly lane?: CachedLane;
}

interface StripValue { readonly key: string; readonly bytes: number; readonly value: unknown }
interface Claim { readonly keys: Set<string>; readonly lost: () => void }
export type StripTileClaim = ReturnType<TileCache['claim']>;
export type StripTileRead = <T>(request: ProductRequest) => Promise<ProductReply<T>>;

export class TileCache {
  private readonly tiles = new Map<string, CachedTile>();
  private readonly pins = new Map<string, number>();
  private readonly stripPins = new Map<string, number>();
  private readonly claims = new Set<Claim>();
  private readonly configs = new Map<string, CachedLane>();
  private nextLane = 0;
  private held = 0;
  readonly cap: number;
  readonly inflight = new Map<string, Promise<void>>();
  readonly joins = new Map<string, () => void>();

  constructor(cap = TILE_CACHE_BYTES) { this.cap = cap; }

  get bytes(): number { return this.held; }
  get size(): number { return this.tiles.size; }
  get stripBytes(): number {
    let bytes = 0;
    for (const key of this.stripPins.keys()) bytes += this.tiles.get(key)?.tile.bytes ?? 0;
    return bytes;
  }

  claim(lost: () => void) {
    const claim: Claim = { keys: new Set(), lost };
    this.claims.add(claim);
    const check = () => {
      if (!this.claims.has(claim)) throw new Error('Timeline tile claim was released');
    };
    const pin = (key: string) => {
      if (claim.keys.has(key)) return;
      claim.keys.add(key);
      this.stripPins.set(key, (this.stripPins.get(key) ?? 0) + 1);
    };
    const cache = this;
    return {
      get active() { return cache.claims.has(claim); },
      keep: (key: string) => {
        check();
        if (!this.tiles.has(key)) return false;
        if (this.held > this.cap) throw new Error('Timeline tiles exceed the tile budget');
        pin(key);
        return true;
      },
      put: (tile: Tile | StripValue, lane?: CachedLane) => {
        check();
        if (lane && this.configs.get(lane.config) !== lane) throw new Error('The chart recording was released');
        const limit = this.cap - tile.bytes + (this.tiles.get(tile.key)?.tile.bytes ?? 0);
        if (this.tiles.has(tile.key)) pin(tile.key);
        this.evict(limit);
        if (this.held > limit) throw new Error('Timeline tiles exceed the tile budget');
        pin(tile.key);
        this.insert(tile, lane);
      },
      value: <T>(key: string): T | undefined => {
        const tile = claim.keys.has(key) ? this.tiles.get(key)?.tile : undefined;
        return tile && 'value' in tile ? tile.value as T : undefined;
      },
      release: () => { this.releaseClaim(claim); this.evict(); },
    };
  }

  acquireLane(config: string): CachedLane {
    let lane = this.configs.get(config);
    if (!lane) {
      lane = { config, id: ++this.nextLane, uses: 0 };
      this.configs.set(config, lane);
    }
    this.retainLane(lane);
    return lane;
  }

  retainLane(lane: CachedLane): void {
    lane.uses++;
  }

  releaseLane(lane: CachedLane): void {
    if (--lane.uses === 0 && this.configs.get(lane.config) === lane) {
      this.configs.delete(lane.config);
    }
  }

  get(key: string): Tile | undefined {
    const tile = this.tiles.get(key);
    if (tile) { this.tiles.delete(key); this.tiles.set(key, tile); }
    return tile && !('value' in tile.tile) ? tile.tile : undefined;
  }

  has(key: string): boolean { return this.tiles.has(key); }

  put(tile: Tile, lane?: CachedLane): void {
    this.insert(tile, lane);
    this.evict();
    if (this.claims.size && this.held > this.cap) {
      for (const claim of this.claims) this.releaseClaim(claim, true);
      this.evict();
    }
  }

  private insert(tile: Tile | StripValue, lane?: CachedLane): void {
    if (lane && this.configs.get(lane.config) !== lane) return;
    if (lane) this.retainLane(lane);
    const before = this.tiles.get(tile.key);
    if (before) this.remove(before);
    this.tiles.set(tile.key, { tile, lane });
    this.held += tile.bytes;
  }

  pin(keys: Iterable<string>): void { for (const key of keys) this.pins.set(key, (this.pins.get(key) ?? 0) + 1); }

  unpin(keys: Iterable<string>): void {
    for (const key of keys) {
      const count = (this.pins.get(key) ?? 0) - 1;
      if (count > 0) this.pins.set(key, count); else this.pins.delete(key);
    }
    this.evict();
  }

  forget(prefix: string): void {
    for (const config of this.configs.keys()) {
      if (config.startsWith(prefix)) this.configs.delete(config);
    }
    for (const [key, tile] of this.tiles) {
      if (!(tile.lane?.config ?? key).startsWith(prefix)) continue;
      if (this.stripPins.has(key)) for (const claim of this.claims) {
        if (claim.keys.has(key)) this.releaseClaim(claim, true);
      }
      this.remove(tile);
    }
  }

  private releaseClaim(claim: Claim, lost = false): void {
    if (!this.claims.delete(claim)) return;
    for (const key of claim.keys) {
      const count = (this.stripPins.get(key) ?? 0) - 1;
      if (count > 0) this.stripPins.set(key, count); else this.stripPins.delete(key);
    }
    claim.keys.clear();
    if (lost) claim.lost();
  }

  private remove(entry: CachedTile): void {
    this.tiles.delete(entry.tile.key);
    this.held -= entry.tile.bytes;
    if (entry.lane) this.releaseLane(entry.lane);
  }

  private evict(limit = this.cap): void {
    if (this.held <= limit) return;
    for (const [key, tile] of this.tiles) {
      if (this.held <= limit) return;
      if (this.pins.has(key) || this.stripPins.has(key)) continue;
      this.remove(tile);
    }
  }
}

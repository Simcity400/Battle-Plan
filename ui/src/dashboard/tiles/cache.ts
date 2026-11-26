import type { Tile } from './tile.ts';

export const TILE_CACHE_BYTES = 64 * 1024 * 1024;

interface CachedLane {
  readonly config: string;
  readonly id: number;
  uses: number;
}

interface CachedTile {
  readonly tile: Tile;
  readonly lane?: CachedLane;
}

export class TileCache {
  private readonly tiles = new Map<string, CachedTile>();
  private readonly pins = new Map<string, number>();
  private readonly configs = new Map<string, CachedLane>();
  private nextLane = 0;
  private held = 0;
  readonly cap: number;
  readonly inflight = new Map<string, Promise<void>>();
  readonly joins = new Map<string, () => void>();

  constructor(cap = TILE_CACHE_BYTES) { this.cap = cap; }

  get bytes(): number { return this.held; }
  get size(): number { return this.tiles.size; }

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
    return tile?.tile;
  }

  has(key: string): boolean { return this.tiles.has(key); }

  put(tile: Tile, lane?: CachedLane): void {
    if (lane && this.configs.get(lane.config) !== lane) return;
    if (lane) this.retainLane(lane);
    const before = this.tiles.get(tile.key);
    if (before) this.remove(before);
    this.tiles.set(tile.key, { tile, lane });
    this.held += tile.bytes;
    this.evict();
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
      if ((tile.lane?.config ?? key).startsWith(prefix)) this.remove(tile);
    }
  }

  private remove(entry: CachedTile): void {
    this.tiles.delete(entry.tile.key);
    this.held -= entry.tile.bytes;
    if (entry.lane) this.releaseLane(entry.lane);
  }

  private evict(): void {
    if (this.held <= this.cap) return;
    for (const [key, tile] of this.tiles) {
      if (this.held <= this.cap) return;
      if (this.pins.has(key)) continue;
      this.remove(tile);
    }
  }
}

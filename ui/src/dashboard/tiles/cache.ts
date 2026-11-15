import type { Tile } from './tile.ts';

/** The most bytes of tile arrays a dashboard keeps. */
export const TILE_CACHE_BYTES = 64 * 1024 * 1024;

/** Tiles by address, least recently used out first, never a tile a view holds (`pin`). The tiles views hold
 * stay even past the cap: they are bounded by what is on screen, and dropping one would only ask it again. */
export class TileCache {
  private readonly tiles = new Map<string, Tile>();
  private readonly pins = new Map<string, number>();
  private held = 0;
  readonly cap: number;
  /** Tiles being asked, by address: a second view needing one waits for the same request. */
  readonly inflight = new Map<string, Promise<void>>();
  /** For a tile in flight that the prepare lane asked: sends its request at once when the page comes to wait on it. */
  readonly joins = new Map<string, () => void>();

  constructor(cap = TILE_CACHE_BYTES) { this.cap = cap; }

  get bytes(): number { return this.held; }
  get size(): number { return this.tiles.size; }

  /** The tile at `key`, now the most recently used. */
  get(key: string): Tile | undefined {
    const tile = this.tiles.get(key);
    if (tile) { this.tiles.delete(key); this.tiles.set(key, tile); }
    return tile;
  }

  has(key: string): boolean { return this.tiles.has(key); }

  put(tile: Tile): void {
    const before = this.tiles.get(tile.key);
    if (before) { this.held -= before.bytes; this.tiles.delete(tile.key); }
    this.tiles.set(tile.key, tile);
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

  /** Forgets every tile whose key starts with `prefix` (a released recording). */
  forget(prefix: string): void {
    for (const [key, tile] of this.tiles) if (key.startsWith(prefix)) { this.tiles.delete(key); this.held -= tile.bytes; }
  }

  private evict(): void {
    if (this.held <= this.cap) return;
    for (const [key, tile] of this.tiles) {
      if (this.held <= this.cap) return;
      if (this.pins.has(key)) continue;
      this.tiles.delete(key);
      this.held -= tile.bytes;
    }
  }
}

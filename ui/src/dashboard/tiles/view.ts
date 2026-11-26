import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary';
import type { Run } from '../../charts/series';
import type { HourSpan, MissingSpan, TilePart } from '../../generated/dashboard';
import { TileCache } from './cache.ts';
import { countryKey } from '../countryKey.ts';
import { tileHead, valueAtHead } from './head.ts';
import { FIRST_BREAK, HAS_BREAK, LEVEL, PRESENT, RUN, SEAM, TILE_BINS, TILE_GROUP, binHoursOf, tileLast, tilesOf } from './tile.ts';
import type { Tile, TileReply } from './tile.ts';

export interface TileViewport { from: number; to: number; end: number; width: number }

export const AHEAD_BINS = 32;

export const HELD_BINS_PER_PIXEL = 4;

export function viewBinHours(hours: number, width: number, held?: { binHours: number; width: number }): number {
  const ideal = binHoursOf(hours, width);
  return held && held.width === width && ideal > held.binHours && hours <= HELD_BINS_PER_PIXEL * width * held.binHours ? held.binHours : ideal;
}

export interface TileLane {
  key: string;
  tags: readonly string[];
  within?: ReadonlyMap<string, readonly HourSpan[]>;
  lastHour: number;
  marks?: boolean;
  ask(binHours: number, index: number, tags: readonly string[], within: Record<string, HourSpan[]> | undefined): Promise<TileReply>;
  prepared?: boolean;
  promote?(binHours: number, index: number, tags: readonly string[], within: Record<string, HourSpan[]> | undefined): void;
  top?: TileTop;
}

export interface TileTop {
  ask(binHours: number, index: number, hour: number): Promise<TileReply>;
}

export interface TileRanked {
  readonly tags: readonly string[];
  readonly hour: number;
  readonly from: number;
  readonly until: number;
  readonly refused: string | null;
}

export interface SpannedLine { summary: Pick<ChartSummary, 'from' | 'to'>; missing: readonly MissingSpan[] }

export interface TiledLine { tag: string; head: string; decimals: number; summary: ChartSummary; missing: MissingSpan[]; parts: TilePart[]; marks: MarkSummary | null }

interface Wanted { binHours: number; first: number; last: number; width: number; cell: number; keys: string[] }
interface Assembled { binHours: number; first: number; last: number; width: number; cell: number }

export const CLUSTER_PIXELS = 18;

export function cellBins(hours: number, width: number, binHours: number, held?: number): number {
  const exact = CLUSTER_PIXELS * hours / (Math.max(1, width) * binHours);
  if (held !== undefined && Math.abs(exact - held) < 0.75) return held;
  return Math.max(1, Math.round(exact));
}

function spansFor(within: TileLane['within'], tag: string, binHours: number, index: number): HourSpan[] | undefined {
  if (!within) return undefined;
  const from = index * TILE_BINS * binHours - binHours, to = (index + 1) * TILE_BINS * binHours - 1;
  return (within.get(tag) ?? []).flatMap(span => span.lastHour < from || span.firstHour > to ? []
    : [{ firstHour: Math.max(from, span.firstHour), lastHour: Math.min(to, span.lastHour) }]);
}

const fresh = (tile: Tile & { askedFor?: number }, lastHour: number) => tile.immutable
  || (tile.askedFor ?? -Infinity) >= lastHour
  || (tile.filled === null ? tile.firstHour > lastHour : tile.filled >= Math.min(tileLast(tile), lastHour));

export class TileView {
  private wanted: Wanted | null = null;
  private lines: ReadonlyMap<string, TiledLine> | null = null;
  private told: { readonly failure: unknown; readonly refused: string | null } = { failure: null, refused: null };
  private readonly made = new WeakMap<TiledLine, { readonly head: string; readonly tiles: readonly Tile[]; readonly from: number; readonly to: number; readonly width: number; readonly cell: number | null }>();
  private assembled: Assembled | null = null;
  private failure: unknown = null;
  private closed = false;
  private ranked: TileRanked | null = null;
  private rankHour: number | null = null;
  private rankAsked = 0;
  private rankPending = false;
  private rankFailedHour: number | null = null;
  private readonly cache: TileCache;
  private readonly lane: TileLane;
  private readonly cacheLane: ReturnType<TileCache['acquireLane']>;
  private readonly changed: () => void;
  asked = 0;

  constructor(cache: TileCache, lane: TileLane, changed: () => void) {
    this.cache = cache; this.lane = lane; this.changed = changed;
    this.cacheLane = cache.acquireLane(lane.key);
  }

  private tileKey(tag: string, binHours: number, index: number): string {
    const spans = spansFor(this.lane.within, tag, binHours, index);
    return `${this.cacheLane.id}${this.lane.marks ? '|marks' : ''}|${binHours}|${index}|${tag}${spans ? `|${JSON.stringify(spans)}` : ''}`;
  }

  get current(): ReadonlyMap<string, TiledLine> | null { return this.lines; }
  get error(): unknown { return this.failure; }
  get top(): TileRanked | null { return this.ranked; }
  get isTop(): boolean { return !!this.lane.top; }

  private get tags(): readonly string[] {
    return this.lane.top ? this.ranked?.tags ?? [] : this.lane.tags;
  }

  setHour(hour: number): void {
    if (this.closed || !this.lane.top) return;
    this.rankHour = hour;
    const held = this.ranked;
    if (held && hour >= held.from && hour <= held.until) return;
    if (this.rankPending || this.rankFailedHour === hour) return;
    this.askTop();
  }

  private askTop(): void {
    const top = this.lane.top, wanted = this.wanted, hour = this.rankHour;
    if (!top || !wanted || hour === null) return;
    const binHours = wanted.binHours, index = Math.max(wanted.first, Math.min(wanted.last, Math.floor(hour / (TILE_BINS * binHours))));
    const asked = ++this.rankAsked;
    this.rankPending = true;
    this.rankFailedHour = null;
    this.asked++;
    void top.ask(binHours, index, hour).then(reply => {
      if (this.closed || asked !== this.rankAsked) return;
      this.rankPending = false;
      this.rankFailedHour = null;
      this.failure = null;
      const ranking = reply.ranking;
      if (!ranking || !Array.isArray(reply.lines)) throw new Error('The chart tile reply names no top.');
      const tags = reply.lines.map(held => countryKey(held.country));
      const tiles = tilesOf(reply, tags, binHours, index, tag => this.tileKey(tag, binHours, index));
      for (const tile of tiles) {
        const held = this.cache.has(tile.key) ? this.cache.get(tile.key) : undefined;
        if (!held || !fresh(held, this.lane.lastHour)) this.cache.put(Object.assign(tile, { askedFor: this.lane.lastHour }), this.cacheLane);
      }
      this.ranked = { tags, hour: ranking.hour, from: ranking.from, until: ranking.until, refused: ranking.refused };
      this.repin();
      this.fetch();
      this.assemble();
      const now = this.rankHour;
      if (now !== null && (now < ranking.from || now > ranking.until)) this.askTop();
    }).catch(error => {
      if (this.closed || asked !== this.rankAsked) return;
      this.rankPending = false;
      this.rankFailedHour = hour;
      this.failure = error;
      this.notify();
    });
  }

  private repin(): void {
    const wanted = this.wanted;
    if (!wanted) return;
    const keys: string[] = [];
    for (let index = wanted.first; index <= wanted.last; index++) for (const tag of this.tags) keys.push(this.tileKey(tag, wanted.binHours, index));
    this.cache.pin(keys);
    this.cache.unpin(wanted.keys);
    wanted.keys = keys;
  }

  setView(view: TileViewport): void {
    if (this.closed || view.to < view.from) return;
    const before = this.wanted;
    const binHours = viewBinHours(view.end - view.from + 1, view.width, before ?? undefined), span = TILE_BINS * binHours;
    const cell = cellBins(view.end - view.from + 1, view.width, binHours, before?.binHours === binHours && before.width === view.width ? before.cell : undefined);
    const first = Math.floor(view.from / span);
    const ahead = Math.min(this.lane.lastHour, view.to + AHEAD_BINS * binHours);
    const last = Math.max(first, Math.floor(Math.max(view.to, ahead) / span));
    const held = this.wanted;
    if (held && held.binHours === binHours && held.first === first && held.last === last && held.width === view.width && held.cell === cell) return;
    const keys: string[] = [];
    for (let index = first; index <= last; index++) for (const tag of this.tags) keys.push(this.tileKey(tag, binHours, index));
    this.cache.pin(keys);
    if (held) this.cache.unpin(held.keys);
    this.wanted = { binHours, first, last, width: view.width, cell, keys };
    if (this.lane.top && !this.ranked && !this.rankPending && this.rankHour !== null && this.rankFailedHour !== this.rankHour) this.askTop();
    this.fetch();
    const made = this.assembled;
    if (made && made.binHours === binHours && made.width === view.width && made.cell === cell && made.first <= first && last <= made.last) return;
    this.assemble();
  }

  retry(): void {
    this.failure = null;
    this.rankFailedHour = null;
    if (this.lane.top && !this.ranked && !this.rankPending && this.rankHour !== null) this.askTop();
    this.fetch();
    this.assemble();
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.wanted) this.cache.unpin(this.wanted.keys);
    this.cache.releaseLane(this.cacheLane);
  }

  private fetch(): void {
    const wanted = this.wanted;
    if (!wanted) return;
    for (let index = wanted.first; index <= wanted.last; index++) {
      const lacking: string[] = [], waiting = new Set<Promise<void>>();
      for (const tag of this.tags) {
        const key = this.tileKey(tag, wanted.binHours, index);
        const tile = this.cache.get(key);
        if (tile && fresh(tile, this.lane.lastHour)) continue;
        const flight = this.cache.inflight.get(key);
        if (!flight) { lacking.push(tag); continue; }
        waiting.add(flight);
        if (!this.lane.prepared) this.cache.joins.get(key)?.();
      }
      for (const flight of waiting) void flight.then(() => this.assemble(), () => {});
      for (let at = 0; at < lacking.length; at += TILE_GROUP) this.request(wanted.binHours, index, lacking.slice(at, at + TILE_GROUP));
    }
  }

  private request(binHours: number, index: number, tags: string[]): void {
    const lane = this.lane, lastHour = lane.lastHour;
    const keys = tags.map(tag => this.tileKey(tag, binHours, index));
    const within = lane.within && Object.fromEntries(tags.map(tag => [tag, spansFor(lane.within, tag, binHours, index)!]));
    this.asked++;
    const flight = lane.ask(binHours, index, tags, within).then(reply => {
      const tiles = tilesOf(reply, tags, binHours, index, tag => keys[tags.indexOf(tag)]);
      for (const tile of tiles) this.cache.put(Object.assign(tile, { askedFor: lastHour }), this.cacheLane);
    });
    this.cache.retainLane(this.cacheLane);
    const join = lane.promote && (() => lane.promote!(binHours, index, tags, within));
    for (const key of keys) {
      this.cache.inflight.set(key, flight);
      if (join) this.cache.joins.set(key, join); else this.cache.joins.delete(key);
    }
    void flight.then(() => this.assemble(), error => {
      if (this.closed) return;
      this.failure = error;
      this.notify();
    }).finally(() => {
      for (const key of keys) if (this.cache.inflight.get(key) === flight) { this.cache.inflight.delete(key); this.cache.joins.delete(key); }
      this.cache.releaseLane(this.cacheLane);
    });
  }

  private assemble(): void {
    const wanted = this.wanted;
    if (this.closed || !wanted) return;
    if (this.lane.top && !this.ranked) return;
    const tiles: Tile[][] = [];
    for (const tag of this.tags) {
      const own: Tile[] = [];
      for (let index = wanted.first; index <= wanted.last; index++) {
        const tile = this.cache.get(this.tileKey(tag, wanted.binHours, index));
        if (!tile) return;
        own.push(tile);
      }
      tiles.push(own);
    }
    const span = TILE_BINS * wanted.binHours, from = wanted.first * span, to = (wanted.last + 1) * span - 1, cell = this.lane.marks === true ? wanted.cell : null;
    const previous = this.lines ? [...this.lines.keys()] : [];
    const lines = new Map<string, TiledLine>();
    let same = this.lines !== null && previous.length === this.tags.length;
    this.tags.forEach((tag, at) => {
      const head = tileHead(this.lane.key, tag), held = this.lines?.get(tag);
      const made = held ? this.made.get(held) : undefined;
      const kept = made && made.head === head && made.from === from && made.to === to && made.width === wanted.width && made.cell === cell
        && made.tiles.length === tiles[at].length && made.tiles.every((tile, index) => tile === tiles[at][index]) ? held! : null;
      if (!kept || previous[at] !== tag) same = false;
      const built = kept ?? line(tag, head, tiles[at], from, to, wanted.width, cell);
      if (!kept) this.made.set(built, { head, tiles: tiles[at], from, to, width: wanted.width, cell });
      lines.set(tag, built);
    });
    this.assembled = { binHours: wanted.binHours, first: wanted.first, last: wanted.last, width: wanted.width, cell: wanted.cell };
    this.failure = null;
    if (same && this.told.failure === null && this.told.refused === (this.ranked?.refused ?? null)) return;
    if (!same) this.lines = lines;
    this.notify();
  }

  private notify(): void {
    this.told = { failure: this.failure, refused: this.ranked?.refused ?? null };
    this.changed();
  }
}

export function tileBuckets(tile: Tile, out: (ChartBucket | null)[] = []): (ChartBucket | null)[] {
  const { flags, values, hours, scale, binHours } = tile;
  let present = 0, v = 0;
  for (let bin = 0; bin < TILE_BINS; bin++) {
    const flag = flags[bin];
    if (!(flag & PRESENT)) { out.push(null); continue; }
    const start = tile.firstHour + bin * binHours, h = present * 5, level = (flag & LEVEL) !== 0;
    const first = values[v] / scale;
    out.push({ firstHour: start + hours[h], lastHour: start + hours[h + 1], lastValueHour: start + hours[h + 2],
      minHour: start + hours[h + 3], maxHour: start + hours[h + 4], firstValue: first, lastValue: level ? first : values[v + 1] / scale,
      minValue: level ? first : values[v + 2] / scale, maxValue: level ? first : values[v + 3] / scale,
      firstBreak: (flag & FIRST_BREAK) !== 0, hasBreak: (flag & HAS_BREAK) !== 0 });
    present++; v += level ? 1 : 4;
  }
  return out;
}

export function tileRuns(tile: Tile, out: Run[] = []): Run[] {
  const { flags, values, scale } = tile;
  let at = 0;
  for (let bin = 0; bin < TILE_BINS; bin++) {
    const flag = flags[bin];
    if (!(flag & PRESENT)) continue;
    const hour = tile.firstHour + bin;
    if (!(flag & RUN)) { out[out.length - 1].lastHour = hour; continue; }
    const value = values[at++] / scale, held = out.at(-1);
    if (flag & SEAM && held && held.lastHour + 1 === hour) held.lastHour = hour;
    else out.push({ firstHour: hour, lastHour: hour, value, breakBefore: (flag & FIRST_BREAK) !== 0 });
  }
  return out;
}

export function hourlyBuckets(runs: readonly Run[], from: number, to: number): (ChartBucket | null)[] {
  const out: (ChartBucket | null)[] = new Array<ChartBucket | null>(to - from + 1).fill(null);
  for (const run of runs) {
    const first = Math.max(from, run.firstHour);
    for (let hour = first; hour <= Math.min(to, run.lastHour); hour++) out[hour - from] = { firstHour: hour, lastHour: hour, lastValueHour: hour,
      minHour: hour, maxHour: hour, firstValue: run.value, lastValue: run.value, minValue: run.value, maxValue: run.value,
      firstBreak: hour === first && run.breakBefore === true, hasBreak: false };
  }
  return out;
}

function tileDrops(tile: Tile, out: [number, number, bigint][]): void {
  if (!tile.marks) return;
  let ordinal = BigInt(tile.marks.before);
  for (const [bin, count] of tile.marks.bins) {
    out.push([tile.firstHour + bin * tile.binHours, count, ordinal]);
    ordinal += BigInt(count);
  }
}

export function cellClusters(drops: readonly (readonly [number, number, bigint])[], binHours: number, cell: number): MarkCluster[] {
  const out: MarkCluster[] = [], hours = binHours * cell;
  let at = 0;
  while (at < drops.length) {
    const index = Math.floor(drops[at][0] / hours), bins: [number, number][] = [];
    let count = 0, end = at;
    while (end < drops.length && Math.floor(drops[end][0] / hours) === index) {
      bins.push([drops[end][0] + binHours - 1, drops[end][1]]);
      count += drops[end][1]; end++;
    }
    out.push({ firstHour: drops[at][0], lastHour: bins[bins.length - 1][0], firstOrdinal: drops[at][2].toString(), count: String(count), bins });
    at = end;
  }
  return out;
}

function line(tag: string, head: string, tiles: readonly Tile[], from: number, to: number, width: number, cell: number | null): TiledLine {
  let buckets: (ChartBucket | null)[] = [];
  const missing: MissingSpan[] = [], parts: TilePart[] = [], seen = new Set<string>(), runs: Run[] = [], drops: [number, number, bigint][] = [];
  const hourly = tiles[0].hourly;
  for (const tile of tiles) {
    if (hourly) tileRuns(tile, runs); else tileBuckets(tile, buckets);
    if (cell !== null) tileDrops(tile, drops);
    for (const span of tile.missing) {
      const held = missing.at(-1);
      if (held && held.reason === span.reason && held.lastHour + 1 === span.firstHour) held.lastHour = span.lastHour;
      else missing.push({ ...span });
    }
    for (const part of tile.parts) {
      const key = `${part.firstHour}:${part.lastHour}:${part.slot}:${part.countryId}`;
      if (!seen.has(key)) { seen.add(key); parts.push(part); }
    }
  }
  const binHours = tiles[0].binHours;
  if (hourly) buckets = hourlyBuckets(runs, from, to);
  const summary: ChartSummary = { from, to, width, buckets, ...(hourly ? { runs } : {}),
    tiles: { binHours, valueAt: hour => valueAtHead(head, summary, binHours, hour) } };
  return { tag, head, decimals: tiles[0].decimals, summary, missing, parts, marks: cell !== null ? { from, to, width, clusters: cellClusters(drops, binHours, cell), tiles: true } : null };
}

export function unrecordedReason(lines: ReadonlyMap<string, SpannedLine> | readonly SpannedLine[] | null | undefined): string | null {
  const all = lines ? [...lines.values()] : [];
  if (!all.length) return null;
  const hours = new Map<string, number>();
  for (const line of all) {
    const { from, to } = line.summary;
    let at = from;
    for (const span of line.missing) {
      if (span.lastHour < at || span.firstHour > to) continue;
      if (span.firstHour > at) return null;
      const end = Math.min(span.lastHour, to);
      hours.set(span.reason, (hours.get(span.reason) ?? 0) + end - at + 1);
      at = end + 1;
      if (at > to) break;
    }
    if (at <= to) return null;
  }
  let best: string | null = null, most = -1;
  for (const [reason, count] of hours) if (count > most) { best = reason; most = count; }
  return best;
}

const caches = new WeakMap<object, TileCache>();
export function tileCache(client: object): TileCache {
  client = (client as { base?: object }).base ?? client;
  let cache = caches.get(client);
  if (!cache) caches.set(client, cache = new TileCache());
  return cache;
}

import type { MissingSpan, SeriesTileReply, TilePart } from '../../generated/dashboard';

/** Bins in a tile. */
export const TILE_BINS = 256;
export const PRESENT = 1, FIRST_BREAK = 2, HAS_BREAK = 4, LEVEL = 8, RUN = 16, SEAM = 32;
/** Tags asked in one tile request at most: a reply's cost follows its keys, so this bounds it under the 16 ms gate
 * (measured: 100 keys 7.0 ms worst on one thread; groups of this size side by side on four cores, see the P4b report). */
export const TILE_GROUP = 64;
export const BASE_HOURS = 6, FANOUT = 16;

export const bucketHours = (level: number) => BASE_HOURS * FANOUT ** level;

/** The bin hours a view of `hours` hours `width` pixels wide reads, as the service picks them: whole buckets of
 * one summary level, or walked hours below a bucket. */
export function binHoursOf(hours: number, width: number): number {
  const perPixel = Math.max(1, Math.floor(hours / Math.max(1, width)));
  if (perPixel < BASE_HOURS) return perPixel;
  let level = 0;
  while (bucketHours(level + 1) <= perPixel) level++;
  const size = bucketHours(level);
  return Math.floor(perPixel / size) * size;
}

/** One tag's tile: its bins as typed arrays, the hours it holds no value in and the holders it meets. */
export interface Tile {
  key: string;
  tag: string;
  binHours: number;
  index: number;
  /** The first hour of its first bin. */
  firstHour: number;
  immutable: boolean;
  /** The last hour of the tile the timeline reaches; null when it reaches none. */
  filled: number | null;
  /** Per bin: PRESENT, FIRST_BREAK, HAS_BREAK, LEVEL; in an hourly tile PRESENT, RUN, FIRST_BREAK, SEAM. */
  flags: Uint8Array;
  /** Its bins are hours, carried as runs: one value per RUN, no hours. */
  hourly: boolean;
  /** Per present bin: first, last, least, greatest, or one value for a LEVEL bin. Raw fixed-point integers divided
   * by `scale`, or values. */
  values: Int32Array | Float64Array;
  scale: number;
  /** The places of the series' fixed-point values, as its replies and followed answers carry them. */
  decimals: number;
  /** Per present bin: first, last, last value's, least's and greatest's hour, less the bin's first hour. */
  hours: Uint8Array | Uint16Array | Uint32Array;
  missing: readonly MissingSpan[];
  parts: readonly TilePart[];
  /** Asked with marks: the drops before the tile, and per bin with any (bin, count). */
  marks?: { before: string; bins: readonly (readonly [number, number])[] };
  /** Bytes its arrays hold. */
  bytes: number;
}

const malformed = () => new Error('The chart tile reply is malformed.');

/** Base64 `text` as bytes. */
export function bytesOf(text: string): Uint8Array {
  const native = (Uint8Array as unknown as { fromBase64?: (text: string) => Uint8Array }).fromBase64;
  if (native) return native(text);
  const binary = atob(text);
  const out = new Uint8Array(binary.length);
  for (let at = 0; at < binary.length; at++) out[at] = binary.charCodeAt(at);
  return out;
}

/** A parsed reply with a tile's bins decoded in place, and the buffer to hand over with it; none for any other. */
export function decodeTileBins(reply: unknown): ArrayBuffer[] {
  const data = (reply as { product?: unknown; data?: { bins?: unknown } } | null)?.data;
  if ((reply as { product?: unknown } | null)?.product !== 'series-tiles' || !data || typeof data.bins !== 'string') return [];
  const bytes = bytesOf(data.bins);
  data.bins = bytes;
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? [bytes.buffer as ArrayBuffer] : [];
}

/** A tile reply whose bins the reply parser has decoded already. */
export type TileReply = Omit<SeriesTileReply, 'bins'> & { bins: string | Uint8Array };

/** A tile reply for `tags` (in the order asked) as one tile per tag, each owning its own arrays. `key` names a
 * tag's tile address. */
export function tilesOf(reply: TileReply, tags: readonly string[], binHours: number, index: number, key: (tag: string) => string): Tile[] {
  if (!reply || typeof reply !== 'object' || reply.binHours !== binHours || reply.index !== index || !Number.isSafeInteger(reply.firstHour)
    || reply.firstHour !== index * TILE_BINS * binHours || !Array.isArray(reply.lines) || reply.lines.length !== tags.length
    || !(typeof reply.bins === 'string' || reply.bins instanceof Uint8Array) || !Number.isInteger(reply.decimals) || reply.decimals < 0 || reply.decimals > 12
    || (reply.filled !== null && !Number.isSafeInteger(reply.filled))) throw malformed();
  const bytes = typeof reply.bins === 'string' ? bytesOf(reply.bins) : reply.bins;
  const count = tags.length * TILE_BINS;
  if (bytes.length < count) throw malformed();
  const flags = bytes.subarray(0, count);
  const words = <T>(make: new (buffer: ArrayBuffer, offset: number, length: number) => T, at: number, length: number) =>
    new make(bytes.buffer as ArrayBuffer, bytes.byteOffset + at, length);
  const scale = reply.rawValues ? 10 ** reply.decimals : 1;
  const line = (at: number) => {
    const held = reply.lines[at];
    if (!held || held.tag !== tags[at] || !Array.isArray(held.missing) || !Array.isArray(held.parts)) throw malformed();
    return { key: key(tags[at]), tag: tags[at], binHours, index, firstHour: reply.firstHour, immutable: reply.immutable, filled: reply.filled,
      scale, decimals: reply.decimals, missing: held.missing, parts: held.parts, ...(held.marks ? { marks: held.marks } : {}) };
  };
  if (reply.hourBytes === 0) {
    if (binHours !== 1) throw malformed();
    // Hourly runs: one value per run start.
    const starts = new Uint32Array(tags.length + 1);
    for (let at = 0; at < count; at++) if (flags[at] & RUN) starts[Math.floor(at / TILE_BINS) + 1]++;
    for (let tag = 0; tag < tags.length; tag++) starts[tag + 1] += starts[tag];
    const valueBytes = reply.rawValues ? 4 : 8;
    if (bytes.length !== count + starts[tags.length] * valueBytes || (bytes.byteOffset + count) % valueBytes) throw malformed();
    const values = reply.rawValues ? words(Int32Array, count, starts[tags.length]) : words(Float64Array, count, starts[tags.length]);
    return tags.map((_, at) => {
      const own = { flags: flags.slice(at * TILE_BINS, (at + 1) * TILE_BINS), values: values.slice(starts[at], starts[at + 1]), hours: new Uint8Array(0) };
      return { ...line(at), ...own, hourly: true, bytes: own.flags.byteLength + own.values.byteLength };
    });
  }
  // Per tag, from its first: present bins, and values.
  const present = new Uint32Array(tags.length + 1), counts = new Uint32Array(tags.length + 1);
  for (let at = 0; at < count; at++) {
    const flag = flags[at];
    if (!(flag & PRESENT)) continue;
    const tag = Math.floor(at / TILE_BINS) + 1;
    present[tag]++; counts[tag] += flag & LEVEL ? 1 : 4;
  }
  for (let tag = 0; tag < tags.length; tag++) { present[tag + 1] += present[tag]; counts[tag + 1] += counts[tag]; }
  const valueBytes = reply.rawValues ? 4 : 8, hourBytes = reply.hourBytes;
  if (hourBytes !== 1 && hourBytes !== 2 && hourBytes !== 4) throw malformed();
  const total = present[tags.length], hoursAt = count + counts[tags.length] * valueBytes;
  if (bytes.length !== hoursAt + total * 5 * hourBytes) throw malformed();
  // Sections start at multiples of their word: the flags fill whole tiles, values are 4 or 8 bytes.
  if ((bytes.byteOffset + count) % valueBytes || (bytes.byteOffset + hoursAt) % hourBytes) throw malformed();
  const values = reply.rawValues ? words(Int32Array, count, counts[tags.length]) : words(Float64Array, count, counts[tags.length]);
  const hours = hourBytes === 1 ? bytes.subarray(hoursAt) : hourBytes === 2 ? words(Uint16Array, hoursAt, total * 5) : words(Uint32Array, hoursAt, total * 5);
  return tags.map((_, at) => {
    const own = {
      flags: flags.slice(at * TILE_BINS, (at + 1) * TILE_BINS),
      values: values.slice(counts[at], counts[at + 1]),
      hours: hours.slice(present[at] * 5, present[at + 1] * 5),
    };
    return { ...line(at), ...own, hourly: false, bytes: own.flags.byteLength + own.values.byteLength + own.hours.byteLength };
  });
}

/** The last hour of `tile`. */
export const tileLast = (tile: Pick<Tile, 'firstHour' | 'binHours'>) => tile.firstHour + TILE_BINS * tile.binHours - 1;

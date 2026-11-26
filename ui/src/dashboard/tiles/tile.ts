import { countryKey } from '../countryKey.ts';
import type { MissingSpan, SeriesTileReply, TilePart } from '../../generated/dashboard';

export const TILE_BINS = 256;
export const PRESENT = 1, FIRST_BREAK = 2, HAS_BREAK = 4, LEVEL = 8, RUN = 16, SEAM = 32;
export const TILE_GROUP = 64;
export const BASE_HOURS = 6, FANOUT = 16;

export const bucketHours = (level: number) => BASE_HOURS * FANOUT ** level;

export function binHoursOf(hours: number, width: number): number {
  const perPixel = Math.max(1, Math.floor(hours / Math.max(1, width)));
  if (perPixel < BASE_HOURS) return perPixel;
  let level = 0;
  while (bucketHours(level + 1) <= perPixel) level++;
  const size = bucketHours(level);
  return Math.floor(perPixel / size) * size;
}

export interface Tile {
  key: string;
  tag: string;
  binHours: number;
  index: number;
  firstHour: number;
  immutable: boolean;
  filled: number | null;
  flags: Uint8Array;
  hourly: boolean;
  values: Int32Array | Float64Array;
  scale: number;
  decimals: number;
  hours: Uint8Array | Uint16Array | Uint32Array;
  missing: readonly MissingSpan[];
  partial: readonly MissingSpan[];
  parts: readonly TilePart[];
  marks?: { before: string; bins: readonly (readonly [number, number])[] };
  bytes: number;
}

const malformed = () => new Error('The chart tile reply is malformed.');

export function bytesOf(text: string): Uint8Array {
  const native = (Uint8Array as unknown as { fromBase64?: (text: string) => Uint8Array }).fromBase64;
  if (native) return native(text);
  const binary = atob(text);
  const out = new Uint8Array(binary.length);
  for (let at = 0; at < binary.length; at++) out[at] = binary.charCodeAt(at);
  return out;
}

export function decodeTileBins(reply: unknown): ArrayBuffer[] {
  const data = (reply as { product?: unknown; data?: { bins?: unknown } } | null)?.data;
  if ((reply as { product?: unknown } | null)?.product !== 'series-tiles' || !data || typeof data.bins !== 'string') return [];
  const bytes = bytesOf(data.bins);
  data.bins = bytes;
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? [bytes.buffer as ArrayBuffer] : [];
}

export type TileReply = Omit<SeriesTileReply, 'bins'> & { bins: string | Uint8Array };

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
    if (!held || countryKey(held.country) !== tags[at] || !Array.isArray(held.missing) || (held.partial !== undefined && !Array.isArray(held.partial)) || !Array.isArray(held.parts)) throw malformed();
    return { key: key(tags[at]), tag: tags[at], binHours, index, firstHour: reply.firstHour, immutable: reply.immutable, filled: reply.filled,
      scale, decimals: reply.decimals, missing: held.missing, partial: held.partial ?? [], parts: held.parts, ...(held.marks ? { marks: held.marks } : {}) };
  };
  if (reply.hourBytes === 0) {
    if (binHours !== 1) throw malformed();
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

export const tileLast = (tile: Pick<Tile, 'firstHour' | 'binHours'>) => tile.firstHour + TILE_BINS * tile.binHours - 1;

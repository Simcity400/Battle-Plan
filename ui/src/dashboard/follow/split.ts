import { DATED_FOLLOW, DATED_PANEL, DATED_PANEL_VERSION, followFailure, wireKind } from './message.ts';
import type { FromPull } from '../../draw/laneMessages';

export const CELL_BIT = 0x8000_0000;
export const laneOf = (id: number, lanes: number) => ((id & ~CELL_BIT) >>> 0) % lanes;

export type Split =
  | { kind: 'failure'; reason: string }
  | { kind: 'follow' | 'panels'; lanes: (Uint8Array | null)[]; owned: number[]; header: number };

const aligned = (at: number) => (at + 7) & ~7;

export function charge(bytes: Uint8Array): number {
  const kind = wireKind(bytes), data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (kind === DATED_PANEL && bytes.byteLength >= 24 && data.getUint16(4, true) === DATED_PANEL_VERSION) return 2 * bytes.byteLength - 24;
  if (kind !== DATED_FOLLOW) return bytes.byteLength;
  let at = 24, total = 24;
  while (at + 16 <= bytes.byteLength) {
    const length = data.getUint32(at + 8, true);
    total += 16 + 2 * Math.max(length, data.getUint32(at + 12, true));
    at += 16 + length;
  }
  return total;
}
const COMPLETED = 4;

export const packet = (kind: 'follow' | 'panels', bytes: Uint8Array, resets: number | undefined): FromPull =>
  kind === 'follow' ? { kind, bytes, resets: resets ?? 0 } : { kind, bytes };

export function resetStamps() {
  const held = new Map<number, { seek: number; count: number }>();
  return (bytes: Uint8Array): number | undefined => {
    if (bytes.byteLength < 24 || wireKind(bytes) !== DATED_FOLLOW) return undefined;
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), flags = data.getUint32(4, true), seek = data.getUint32(16, true);
    const at = held.get(flags >>> 1) ?? { seek, count: 0 };
    if (seek < at.seek) return at.count;
    at.seek = seek;
    if (flags & 1) at.count++;
    held.set(flags >>> 1, at);
    return at.count;
  };
}

export function splitForLanes(input: Uint8Array | ArrayBuffer, lanes: number, stats?: { recordVisits: number }): Split {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  const failure = followFailure(bytes);
  if (failure !== null) return { kind: 'failure', reason: failure };
  const kind = wireKind(bytes);
  if (kind !== DATED_FOLLOW && kind !== DATED_PANEL) throw new Error(`No followed message kind ${kind.toString(16)}.`);
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 24) throw new Error('A followed message ends inside its header.');
  const follow = kind === DATED_FOLLOW;
  if (!follow && data.getUint16(4, true) !== DATED_PANEL_VERSION) throw new Error(`Dated panels of version ${data.getUint16(4, true)} are not supported; this page reads version ${DATED_PANEL_VERSION}.`);
  const count = data.getUint32(follow ? 12 : 8, true);
  const records: { lane: number | 'all'; start: number; end: number }[] = [];
  let at = 24;
  for (let i = 0; i < count; i++) {
    if (at + 16 > bytes.byteLength) throw new Error('A followed message ends inside a record.');
    const id = data.getUint32(at, true), length = data.getUint32(at + (follow ? 8 : 12), true);
    const end = follow ? at + 16 + length : aligned(at + 16 + length);
    if (end > bytes.byteLength) throw new Error('A followed message ends inside a record.');
    const route = !follow && data.getUint8(at + 8) === COMPLETED;
    records.push({ lane: route ? 'all' : laneOf(id, lanes), start: at, end });
    at = end;
  }
  if (at !== bytes.byteLength) throw new Error('A followed message holds bytes past its records.');
  const owned = Array.from({ length: lanes }, () => 0);
  const sizes = Array.from({ length: lanes }, () => 24), counts = Array.from({ length: lanes }, () => 0);
  let shared = 0;
  for (const record of records) {
    const size = record.end - record.start;
    if (record.lane === 'all') { shared += size; for (let lane = 0; lane < lanes; lane++) { sizes[lane]! += size; counts[lane]!++; } }
    else { sizes[record.lane]! += size; counts[record.lane]!++; owned[record.lane]! += follow ? 16 + 2 * Math.max(size - 16, data.getUint32(record.start + 12, true)) : 2 * size; }
  }
  const whole = input instanceof ArrayBuffer ? counts.findIndex(n => n === count && (follow || count > 0)) : -1;
  const out: (Uint8Array | null)[] = sizes.map((size, index) => index !== whole && (follow || counts[index]) ? new Uint8Array(size) : null);
  const put = sizes.map(() => 24);
  out.forEach((lane, index) => {
    if (!lane) return;
    lane.set(bytes.subarray(0, 24));
    new DataView(lane.buffer).setUint32(follow ? 12 : 8, counts[index]!, true);
  });
  for (const record of records) {
    if (record.lane === whole) continue;
    const piece = bytes.subarray(record.start, record.end);
    if (record.lane === 'all') {
      if (stats) stats.recordVisits += lanes;
      for (let lane = 0; lane < lanes; lane++) { out[lane]?.set(piece, put[lane]!); put[lane]! += piece.length; }
    } else {
      if (stats) stats.recordVisits++;
      const lane = record.lane;
      out[lane]!.set(piece, put[lane]!); put[lane]! += piece.length;
    }
  }
  if (whole >= 0) out[whole] = bytes;
  return { kind: follow ? 'follow' : 'panels', lanes: out, owned, header: 24 + 2 * shared };
}

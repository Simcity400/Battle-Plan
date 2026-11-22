import { DATED_FOLLOW, DATED_PANEL, followFailure, wireKind } from './message.ts';

export const CELL_BIT = 0x8000_0000;
export const laneOf = (id: number, lanes: number) => ((id & ~CELL_BIT) >>> 0) % lanes;

export type Split =
  | { kind: 'failure'; reason: string }
  | { kind: 'follow' | 'panels'; lanes: (Uint8Array | null)[]; owned: number[]; header: number };

const aligned = (at: number) => (at + 7) & ~7;
const COMPLETED = 4;

export function splitForLanes(bytes: Uint8Array, lanes: number): Split {
  const failure = followFailure(bytes);
  if (failure !== null) return { kind: 'failure', reason: failure };
  const kind = wireKind(bytes);
  if (kind !== DATED_FOLLOW && kind !== DATED_PANEL) throw new Error(`No followed message kind ${kind.toString(16)}.`);
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 24) throw new Error('A followed message ends inside its header.');
  const follow = kind === DATED_FOLLOW;
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
  let shared = 24;
  for (const record of records) {
    const size = record.end - record.start;
    if (record.lane === 'all') { shared += size; for (let lane = 0; lane < lanes; lane++) { sizes[lane]! += size; counts[lane]!++; } }
    else { sizes[record.lane]! += size; counts[record.lane]!++; owned[record.lane]! += size; }
  }
  const out = sizes.map(size => new Uint8Array(size)), put = sizes.map(() => 24);
  out.forEach((lane, index) => {
    lane.set(bytes.subarray(0, 24));
    new DataView(lane.buffer).setUint32(follow ? 12 : 8, counts[index]!, true);
  });
  for (const record of records) {
    const piece = bytes.subarray(record.start, record.end);
    for (let lane = 0; lane < lanes; lane++) if (record.lane === 'all' || record.lane === lane) { out[lane]!.set(piece, put[lane]!); put[lane]! += piece.length; }
  }
  return { kind: follow ? 'follow' : 'panels', lanes: out.map((lane, index) => follow || counts[index] ? lane : null), owned, header: shared };
}

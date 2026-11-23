import { encode } from '@msgpack/msgpack';

export type Spec =
  | { notRecorded: string }
  | { keys: bigint[]; columns: number; values: bigint[]; states?: number[]; reasons?: (string | null)[]; names?: (string | null)[]; hours?: number[]; changed?: boolean };

class Bytes {
  bytes: number[] = [];
  u8(value: number) { this.bytes.push(value & 0xff); }
  u16(value: number) { this.u8(value); this.u8(value >> 8); }
  u32(value: number) { for (let i = 0; i < 4; i++) this.u8(Number((BigInt(value) >> BigInt(8 * i)) & 0xffn)); }
  u64(value: bigint) { for (let i = 0; i < 8; i++) this.u8(Number((BigInt.asUintN(64, value) >> BigInt(8 * i)) & 0xffn)); }
  pad() { while (this.bytes.length % 8) this.u8(0); }
  texts(texts: readonly string[]) {
    const encoded = texts.map(text => new TextEncoder().encode(text));
    let end = 0;
    this.u32(0);
    for (const bytes of encoded) { end += bytes.length; this.u32(end); }
    this.pad();
    for (const bytes of encoded) bytes.forEach(byte => this.u8(byte));
    this.pad();
  }
}

export const answerBody = (spec: Spec) => new Uint8Array(body(spec));

function body(spec: Spec): number[] {
  const out = new Bytes();
  if ('notRecorded' in spec) {
    out.u8(0); out.u8(0); out.u16(0); out.u32(0); out.u32(0); out.u32(1);
    out.texts([spec.notRecorded]);
    return out.bytes;
  }
  const rows = spec.keys.length;
  const texts: string[] = [], held = new Map<string, number>();
  const index = (text: string | null | undefined) => {
    if (text === null || text === undefined) return 0xffff_ffff;
    let at = held.get(text);
    if (at === undefined) held.set(text, at = texts.push(text) - 1);
    return at;
  };
  const reasons: number[] = [], names: number[] = [];
  for (let row = 0; row < rows; row++) {
    reasons.push(index(spec.reasons?.[row]));
    names.push(index(spec.names?.[row]));
  }
  const ascending = !spec.hours && spec.keys.every((key, at) => at === 0 || spec.keys[at - 1]! < key);
  out.u8(spec.hours ? 2 : spec.changed ? 3 : 1); out.u8(ascending ? 1 : 0); out.u16(0); out.u32(rows); out.u32(spec.columns); out.u32(texts.length);
  spec.keys.forEach(key => out.u64(key));
  spec.keys.forEach(key => out.u64(key >> 64n));
  spec.values.forEach(value => out.u64(value));
  if (spec.hours) { spec.hours.forEach(hour => out.u32(hour)); out.pad(); }
  for (let at = 0; at < rows * spec.columns; at++) out.u8(spec.states?.[at] ?? 0);
  out.pad();
  for (let row = 0; row < rows; row++) out.u32(reasons[row] ?? 0xffff_ffff);
  out.pad();
  for (let row = 0; row < rows; row++) out.u32(names[row] ?? 0xffff_ffff);
  out.pad();
  out.texts(texts);
  return out.bytes;
}

export function message(hour: number, answers: { id: number; hour?: number; answer: Spec; held?: number }[], reset = false, route = 0, seek = 0): Uint8Array {
  const out = new Bytes();
  out.u32(0x334c4f46); out.u32(route * 2 + (reset ? 1 : 0)); out.u32(hour); out.u32(answers.length); out.u32(seek); out.u32(0);
  for (const { id, hour: at, answer, held: whole } of answers) {
    const held = body(answer);
    out.u32(id); out.u32(at ?? hour); out.u32(held.length); out.u32(whole ?? 0);
    held.forEach(byte => out.u8(byte));
  }
  return new Uint8Array(out.bytes);
}

export function datedPanel(id: number, hour: number, data?: unknown, seek = 1, route = 9) {
  const body = data === undefined ? new Uint8Array() : encode(data);
  const out = new Uint8Array((40 + body.length + 7) & ~7), words = new Uint32Array(out.buffer);
  words.set([0x324c4e50, 6, 1, 0, route, seek, id, hour, data === undefined ? 3 : 1, body.length]);
  out.set(body, 40);
  return out;
}

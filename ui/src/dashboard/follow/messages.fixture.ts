import { encode } from '@msgpack/msgpack';
import { parseFollowed } from './message.ts';
import type { PageReading } from '../../draw/followed';

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
  text(text: string) { const bytes = new TextEncoder().encode(text); this.u32(bytes.length); bytes.forEach(byte => this.u8(byte)); }
}

function body(spec: Spec): number[] {
  const out = new Bytes();
  if ('notRecorded' in spec) {
    out.u8(0); out.u8(0); out.u16(0); out.u32(0); out.u32(0); out.u32(1);
    out.text(spec.notRecorded); out.pad();
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
  out.u8(spec.hours ? 2 : spec.changed ? 3 : 1); out.u8(0); out.u16(0); out.u32(rows); out.u32(spec.columns); out.u32(texts.length);
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
  texts.forEach(text => out.text(text)); out.pad();
  return out.bytes;
}

export function message(hour: number, answers: { id: number; hour?: number; answer: Spec }[], reset = false, route = 0, seek = 0): Uint8Array {
  const out = new Bytes();
  out.u32(0x324c4f46); out.u32(route * 2 + (reset ? 1 : 0)); out.u32(hour); out.u32(answers.length); out.u32(seek); out.u32(0);
  for (const { id, hour: at, answer } of answers) {
    const held = body(answer);
    out.u32(id); out.u32(at ?? hour); out.u32(held.length); out.u32(0);
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

export function fakeTransport() {
  const sent: string[] = [];
  let receive: ((reading: PageReading) => void) | null = null;
  return {
    sent,
    follow: (id: number, query: unknown) => { sent.push(`follow ${id} ${JSON.stringify(query)}`); },
    unfollow: (id: number) => { sent.push(`unfollow ${id}`); },
    listen: (handler: (reading: PageReading) => void) => { receive = handler; return () => { receive = null; }; },
    deliver(bytes: Uint8Array) {
      const step = parseFollowed(bytes);
      for (const { id, hour, answer } of step.answers) receive?.({ id, hour, answer, through: step.hour, seek: 0 });
    },
    reading: (reading: PageReading) => { receive?.(reading); },
  };
}

export function manualFrames() {
  const queued: (() => void)[] = [];
  return { schedule: (run: () => void) => { queued.push(run); }, run: () => { const due = queued.splice(0); due.forEach(run => run()); return due.length; }, get pending() { return queued.length; } };
}

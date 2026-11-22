import { DATED_FOLLOW } from '../dashboard/follow/message.ts';

export const RECORDED_FOLLOW = 0x324c4f46;
const aligned = (at: number) => (at + 7) & ~7;

function upgradeBody(body: Uint8Array): Uint8Array {
  const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
  const kind = view.getUint8(0), rows = view.getUint32(4, true), columns = view.getUint32(8, true), count = view.getUint32(12, true);
  let at = 16;
  if (kind !== 0) {
    at += rows * 16 + rows * columns * 8;
    if (kind === 2) at = aligned(at + rows * 4);
    at = aligned(at + rows * columns);
    at = aligned(at + rows * 4);
    at = aligned(at + rows * 4);
  }
  const head = body.subarray(0, at), texts: Uint8Array[] = [];
  for (let i = 0; i < count; i++) {
    const length = view.getUint32(at, true);
    texts.push(body.subarray(at + 4, at + 4 + length));
    at += 4 + length;
  }
  let ascending = kind === 1 || kind === 3;
  for (let row = 1; ascending && row < rows; row++) {
    const key = (r: number) => (view.getBigUint64(16 + (rows + r) * 8, true) << 64n) | view.getBigUint64(16 + r * 8, true);
    if (key(row - 1) >= key(row)) ascending = false;
  }
  const bytes = texts.reduce((sum, text) => sum + text.length, 0);
  const endsAt = head.length, textAt = aligned(endsAt + (count + 1) * 4), out = new Uint8Array(aligned(textAt + bytes));
  out.set(head);
  out[1] = ascending ? 1 : 0;
  const ends = new DataView(out.buffer);
  let end = 0, put = textAt;
  ends.setUint32(endsAt, 0, true);
  texts.forEach((text, index) => { end += text.length; ends.setUint32(endsAt + (index + 1) * 4, end, true); out.set(text, put); put += text.length; });
  return out;
}

export function upgradeRecorded(bytes: Uint8Array): Uint8Array {
  if (bytes.byteLength < 24) return bytes;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0, true) !== RECORDED_FOLLOW) return bytes;
  const parts: Uint8Array[] = [];
  const count = view.getUint32(12, true);
  let at = 24;
  for (let i = 0; i < count; i++) {
    const length = view.getUint32(at + 8, true);
    const body = length ? upgradeBody(bytes.subarray(at + 16, at + 16 + length)) : new Uint8Array();
    const record = new Uint8Array(16 + body.length);
    record.set(bytes.subarray(at, at + 16));
    new DataView(record.buffer).setUint32(8, body.length, true);
    record.set(body, 16);
    parts.push(record);
    at += 16 + length;
  }
  const out = new Uint8Array(24 + parts.reduce((sum, part) => sum + part.length, 0));
  out.set(bytes.subarray(0, 24));
  new DataView(out.buffer).setUint32(0, DATED_FOLLOW, true);
  let put = 24;
  for (const part of parts) { out.set(part, put); put += part.length; }
  return out;
}

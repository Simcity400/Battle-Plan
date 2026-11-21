import { decode } from '@msgpack/msgpack';


const RESET = 1;
const NO_REASON = 0xffff_ffff;

export const CELL = { value: 0, missing: 1, unrecorded: 2, noRow: 3 } as const;

export interface MessageViews {
  u8: Uint8Array;
  u16: Uint16Array;
  u32: Uint32Array;
  i32: Int32Array;
  i64: BigInt64Array;
  u64: BigUint64Array;
}

export interface FollowedRows {
  kind: 'rows' | 'log' | 'changed';
  rows: number;
  columns: number;
  views: MessageViews;
  keys: number;
  values: number;
  hours: number;
  states: number;
  reasons: number;
  names: number;
  texts: Uint32Array;
  decoded: (string | undefined)[];
}

export type FollowedAnswer = { kind: 'notRecorded'; reason: string } | FollowedRows;

export interface FollowedPanel {
  data?: unknown;
  refused: string | null;
}

const FAILURE_MAGIC = 0x314e4546;

export function followFailure(bytes: ArrayBuffer | Uint8Array): string | null {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (view.byteLength < 4 || new DataView(view.buffer, view.byteOffset, 4).getUint32(0, true) !== FAILURE_MAGIC) return null;
  return decoder.decode(view.subarray(4));
}

export interface FollowedMessage {
  limited?: Map<number, number>;
  route: number;
  hour: number;
  reset: boolean;
  seek: number; clock: number;
  answers: { id: number; hour: number; answer: FollowedAnswer; span?: [number, number] }[];
}

const decoder = new TextDecoder();
const aligned = (at: number) => (at + 7) & ~7;

function texts(views: MessageViews, data: DataView, at: number, count: number): [string[], number] {
  const out: string[] = [];
  for (let i = 0; i < count; i++) {
    const length = data.getUint32(at, true);
    out.push(decoder.decode(views.u8.subarray(at + 4, at + 4 + length)));
    at += 4 + length;
  }
  return [out, aligned(at)];
}

function textStarts(data: DataView, at: number, count: number, end: number): [Uint32Array, number] {
  const starts = new Uint32Array(count);
  for (let i = 0; i < count; i++) {
    if (at + 4 > end) throw new Error('A followed answer ends early.');
    starts[i] = at;
    at += 4 + data.getUint32(at, true);
  }
  return [starts, aligned(at)];
}

function textAt(answer: FollowedRows, at: number): string | null {
  if (at >= answer.texts.length) return null;
  let text = answer.decoded[at];
  if (text === undefined) {
    const start = answer.texts[at]!, u8 = answer.views.u8;
    const length = u8[start]! | u8[start + 1]! << 8 | u8[start + 2]! << 16 | u8[start + 3]! << 24;
    text = answer.decoded[at] = decoder.decode(u8.subarray(start + 4, start + 4 + length));
  }
  return text;
}

function body(views: MessageViews, data: DataView, start: number, end: number): FollowedAnswer {
  const kind = data.getUint8(start);
  const rows = data.getUint32(start + 4, true), columns = data.getUint32(start + 8, true), count = data.getUint32(start + 12, true);
  let at = start + 16;
  if (kind === 0) {
    const [[reason]] = texts(views, data, at, count);
    if (reason === undefined) throw new Error('A followed answer that is not recorded gives no reason.');
    return { kind: 'notRecorded', reason };
  }
  if (kind !== 1 && kind !== 2 && kind !== 3) throw new Error(`No followed answer kind ${kind}.`);
  const cells = rows * columns;
  if (at + rows * 24 + cells * 9 > end) throw new Error('A followed answer ends early.');
  const keys = at / 8; at += rows * 16;
  const values = at / 8; at += cells * 8;
  let hours = -1;
  if (kind === 2) { hours = at / 4; at = aligned(at + rows * 4); }
  const states = at; at = aligned(at + cells);
  const reasons = at / 4; at = aligned(at + rows * 4);
  const names = at / 4; at = aligned(at + rows * 4);
  const [starts, next] = textStarts(data, at, count, end);
  if (next !== end) throw new Error('A followed answer holds bytes past its texts.');
  return { kind: kind === 2 ? 'log' : kind === 3 ? 'changed' : 'rows', rows, columns, views, keys, values, hours, states, reasons, names, texts: starts, decoded: [] };
}

function viewsOf(bytes: ArrayBuffer | Uint8Array) {
  let view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (view.byteOffset % 8 !== 0 || view.byteLength % 8 !== 0) view = view.slice();
  const { buffer, byteOffset, byteLength } = view;
  const words = byteLength / 8;
  const views: MessageViews = { u8: new Uint8Array(buffer, byteOffset, byteLength), u16: new Uint16Array(buffer, byteOffset, words * 4),
    u32: new Uint32Array(buffer, byteOffset, words * 2),
    i32: new Int32Array(buffer, byteOffset, words * 2), i64: new BigInt64Array(buffer, byteOffset, words), u64: new BigUint64Array(buffer, byteOffset, words) };
  const data = new DataView(buffer, byteOffset, byteLength);
  return { views, data, byteLength };
}

export function parseAnswer(bytes: Uint8Array): FollowedAnswer {
  const { views, data, byteLength } = viewsOf(bytes);
  return body(views, data, 0, byteLength);
}

export const DATED_FOLLOW = 0x324c4f46, DATED_PANEL = 0x324c4e50;

export function wireKind(bytes: ArrayBuffer | Uint8Array): number {
  const v = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  return v.byteLength < 4 ? 0 : new DataView(v.buffer, v.byteOffset, 4).getUint32(0, true);
}

export function parseFollowed(bytes: ArrayBuffer | Uint8Array): FollowedMessage {
  const { views, data, byteLength } = viewsOf(bytes);
  if (byteLength < 24 || data.getUint32(0, true) !== DATED_FOLLOW) throw new Error('Not a followed message.');
  const flags = data.getUint32(4, true);
  const message: FollowedMessage = { route: flags >>> 1, hour: data.getInt32(8, true), reset: (flags & RESET) !== 0, seek: data.getUint32(16, true), clock: data.getInt32(20, true), answers: [] };
  const count = data.getUint32(12, true);
  let at = 24;
  for (let i = 0; i < count; i++) {
    if (at + 16 > byteLength) throw new Error('A followed message ends early.');
    const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), length = data.getUint32(at + 8, true);
    const start = at + 16, end = start + length;
    if (end > byteLength) throw new Error('A followed message ends early.');
    if (!length) (message.limited ??= new Map()).set(id, hour);
    else message.answers.push({ id, hour, answer: body(views, data, start, end), span: [start, end] });
    at = end;
  }
  if (at !== byteLength) throw new Error('A followed message holds bytes past its answers.');
  return message;
}

export const cellState = (answer: FollowedRows, at: number): number => answer.views.u8[answer.states + at]!;

export const cellValue = (answer: FollowedRows, at: number): bigint => answer.views.i64[answer.values + at]!;

export function cellNumber(answer: FollowedRows, at: number): number | null {
  const word = (answer.values + at) * 2, high = answer.views.i32[word + 1]!;
  return high >= -0x200000 && high < 0x200000 ? high * 0x1_0000_0000 + (answer.views.i32[word]! >>> 0) : null;
}

export function rowReason(answer: FollowedRows, row: number): string | null {
  const at = answer.views.u32[answer.reasons + row]!;
  return at === NO_REASON ? null : textAt(answer, at);
}

export function rowName(answer: FollowedRows, row: number): string | null {
  const at = answer.views.u32[answer.names + row]!;
  return at === NO_REASON ? null : textAt(answer, at);
}

export const rowKey = (answer: FollowedRows, row: number): bigint =>
  (answer.views.u64[answer.keys + answer.rows + row]! << 64n) | answer.views.u64[answer.keys + row]!;

export const rowHour = (answer: FollowedRows, row: number): number | null => answer.hours < 0 ? null : answer.views.i32[answer.hours + row]!;

export function rowKeys(answer: FollowedAnswer | undefined): string {
  if (!answer || answer.kind === 'notRecorded') return '';
  let out = '';
  for (let row = 0; row < answer.rows; row++) out += `${row ? ',' : ''}${rowKey(answer, row)}`;
  return out;
}

export interface DatedPanel { id: number; hour: number; panel: FollowedPanel | null; stepped?: boolean }

export function parseDatedPanels(bytes: Uint8Array) {
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (data.byteLength < 24 || wireKind(bytes) !== DATED_PANEL || data.getUint16(4, true) !== 5) throw new Error('Invalid dated panel header.');
  const count = data.getUint32(8, true), clock = data.getInt32(12, true), route = data.getUint32(16, true), seek = data.getUint32(20, true);
  const panels: DatedPanel[] = [];
  let completed: number | undefined;
  let at = 24;
  for (let i = 0; i < count; i++) {
    const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), kind = data.getUint8(at + 8);
    const length = data.getUint32(at + 12, true), end = at + 16 + length;
    if (end > bytes.length || ![0, 1, 3, 4].includes(kind) || (kind >= 3 && length)) throw new Error('Invalid dated panel record.');
    if (kind === 4) {
      if (id !== 0 || i !== count - 1) throw new Error('Invalid completed product step.');
      completed = hour; at = aligned(end); continue;
    }
    const body = bytes.subarray(at + 16, end);
    panels.push({ id, hour, panel: kind === 3 ? null : kind === 0 ? { refused: decoder.decode(body) } : { data: decode(body), refused: null } });
    if (data.getUint8(at + 9) & 1) panels.at(-1)!.stepped = true;
    at = aligned(end);
  }
  if (at !== bytes.length) throw new Error('Unexpected bytes after dated panels.');
  return { route, clock, seek, panels, completed };
}

export type FollowItem =
  | { kind: 'panels'; batch: ReturnType<typeof parseDatedPanels> }
  | { kind: 'followed'; message: FollowedMessage }
  | { kind: 'failure'; reason: string };

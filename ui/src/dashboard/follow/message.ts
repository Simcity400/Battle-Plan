/** A step's message of followed answers (`crates/dashboard-follow/src/answer.rs`), read once: the
 * numbers stay where they arrived, read through one set of typed views over the whole message,
 * which every answer in it shares (each section starts on an eight-byte boundary). */

const MAGIC = 0x314c4f46;
const RESET = 1;
const NO_REASON = 0xffff_ffff;

/** A cell's state: a value, an optional number without one, not recorded, or no row (its row's
 * reason says why). Never shown as zero unless it is a value. */
export const CELL = { value: 0, missing: 1, unrecorded: 2, noRow: 3 } as const;

/** Typed views over one message. */
export interface MessageViews {
  u8: Uint8Array;
  u16: Uint16Array;
  u32: Uint32Array;
  i32: Int32Array;
  i64: BigInt64Array;
  u64: BigUint64Array;
}

export interface FollowedRows {
  /** Every row asked, rows appended since the push before, or a live query's rows that changed since the push before
   * (laid over the rows held, by key). */
  kind: 'rows' | 'log' | 'changed';
  rows: number;
  columns: number;
  views: MessageViews;
  /** Where its sections start in `views`: keys' low halves in `u64` (the high halves follow),
   * cells row by row in `i64`, a log's insert hours in `i32` (-1 for rows), cell states in
   * `u8`, each row's reason and each row's name (a series' tag), indexes into its texts, in `u32`. */
  keys: number;
  values: number;
  hours: number;
  states: number;
  reasons: number;
  names: number;
  /** Where each text starts in `views.u8` (its u32 length first): decoded only when read ([`rowReason`]). */
  texts: Uint32Array;
  decoded: (string | undefined)[];
}

export type FollowedAnswer = { kind: 'notRecorded'; reason: string } | FollowedRows;

/** A panel at the head (`crates/dashboard-follow/src/panel.rs`): the product's reply as JSON, or why it has none
 * at this hour. `dataText` is the reply as it arrived, what "differs" compares. */
export interface FollowedPanel {
  data?: unknown;
  dataText?: string;
  refused: string | null;
}

/** `PNL1`, a panel message's first four bytes, as a little-endian u32. */
const PANEL_MAGIC = 0x314c4e50;

/** Whether `bytes` is a message of text panels rather than of followed answers. */
export function isPanelMessage(bytes: ArrayBuffer | Uint8Array): boolean {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  return view.byteLength >= 4 && new DataView(view.buffer, view.byteOffset, 4).getUint32(0, true) === PANEL_MAGIC;
}

/** A message of panels: each panel's id and hour, and its reply or its refusal. */
export function parsePanels(bytes: ArrayBuffer | Uint8Array): { id: number; hour: number; panel: FollowedPanel }[] {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (!isPanelMessage(view)) throw new Error('Not a panel message.');
  const body = JSON.parse(decoder.decode(view.subarray(4))) as { panels?: unknown };
  if (!Array.isArray(body.panels)) throw new Error('A panel message names no panels.');
  return body.panels.map(item => {
    const { id, hour, refused, data } = item as Record<string, unknown>;
    if (!Number.isSafeInteger(id) || !Number.isSafeInteger(hour) || !(refused === null || typeof refused === 'string') || (refused === null && data == null)) {
      throw new Error('A panel in the message is malformed.');
    }
    const panel: FollowedPanel = refused === null ? { data, dataText: JSON.stringify(data), refused } : { refused };
    return { id: id as number, hour: hour as number, panel };
  });
}

/** `FEN1`: the first four bytes of the notice that pushed answers stopped (`crates/dashboard-follow/src/push.rs`, `failure`). */
const FAILURE_MAGIC = 0x314e4546;

/** Why pushed answers stopped, when `bytes` is the notice that they did. */
export function followFailure(bytes: ArrayBuffer | Uint8Array): string | null {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (view.byteLength < 4 || new DataView(view.buffer, view.byteOffset, 4).getUint32(0, true) !== FAILURE_MAGIC) return null;
  return decoder.decode(view.subarray(4));
}

export interface FollowedMessage {
  /** The follow (one playhead's) that sent it. */
  route: number;
  hour: number;
  /** The settled answers after a seek: logs start again. */
  reset: boolean;
  answers: { id: number; hour: number; answer: FollowedAnswer }[];
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

/** Where each of `count` texts starts, and the end past them. */
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

/** `bytes` read as a followed message; bytes not starting on an eight-byte boundary are copied
 * once so the views can lie over them. */
export function parseFollowed(bytes: ArrayBuffer | Uint8Array): FollowedMessage {
  let view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (view.byteOffset % 8 !== 0 || view.byteLength % 8 !== 0) view = view.slice();
  const { buffer, byteOffset, byteLength } = view;
  const words = byteLength / 8;
  const views: MessageViews = { u8: new Uint8Array(buffer, byteOffset, byteLength), u16: new Uint16Array(buffer, byteOffset, words * 4),
    u32: new Uint32Array(buffer, byteOffset, words * 2),
    i32: new Int32Array(buffer, byteOffset, words * 2), i64: new BigInt64Array(buffer, byteOffset, words), u64: new BigUint64Array(buffer, byteOffset, words) };
  const data = new DataView(buffer, byteOffset, byteLength);
  if (byteLength < 16 || data.getUint32(0, true) !== MAGIC) throw new Error('Not a followed message.');
  const flags = data.getUint32(4, true);
  const message: FollowedMessage = { route: flags >>> 1, hour: data.getInt32(8, true), reset: (flags & RESET) !== 0, answers: [] };
  const count = data.getUint32(12, true);
  let at = 16;
  for (let i = 0; i < count; i++) {
    if (at + 16 > byteLength) throw new Error('A followed message ends early.');
    const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), length = data.getUint32(at + 8, true);
    const start = at + 16, end = start + length;
    if (end > byteLength) throw new Error('A followed message ends early.');
    message.answers.push({ id, hour, answer: body(views, data, start, end) });
    at = end;
  }
  if (at !== byteLength) throw new Error('A followed message holds bytes past its answers.');
  return message;
}

/** Cell `at` (row by row)'s state. */
export const cellState = (answer: FollowedRows, at: number): number => answer.views.u8[answer.states + at]!;

/** Cell `at`'s raw value. */
export const cellValue = (answer: FollowedRows, at: number): bigint => answer.views.i64[answer.values + at]!;

/** Cell `at` as a number when it is a safe integer, read without making a big integer; null
 * when only [`cellValue`] holds it. */
export function cellNumber(answer: FollowedRows, at: number): number | null {
  const word = (answer.values + at) * 2, high = answer.views.i32[word + 1]!;
  return high >= -0x200000 && high < 0x200000 ? high * 0x1_0000_0000 + (answer.views.i32[word]! >>> 0) : null;
}

/** Row `row`'s reason, if it has one. */
export function rowReason(answer: FollowedRows, row: number): string | null {
  const at = answer.views.u32[answer.reasons + row]!;
  return at === NO_REASON ? null : textAt(answer, at);
}

/** Row `row`'s name (a series' tag), if it has one. */
export function rowName(answer: FollowedRows, row: number): string | null {
  const at = answer.views.u32[answer.names + row]!;
  return at === NO_REASON ? null : textAt(answer, at);
}

/** Row `row`'s key. */
export const rowKey = (answer: FollowedRows, row: number): bigint =>
  (answer.views.u64[answer.keys + answer.rows + row]! << 64n) | answer.views.u64[answer.keys + row]!;

/** A log's row `row`: the hour it was inserted. */
export const rowHour = (answer: FollowedRows, row: number): number | null => answer.hours < 0 ? null : answer.views.i32[answer.hours + row]!;

/** The rows' keys in order: what a table's structure is. */
export function rowKeys(answer: FollowedAnswer | undefined): string {
  if (!answer || answer.kind === 'notRecorded') return '';
  let out = '';
  for (let row = 0; row < answer.rows; row++) out += `${row ? ',' : ''}${rowKey(answer, row)}`;
  return out;
}

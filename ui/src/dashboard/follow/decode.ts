import { DATED_PANEL, followFailure, parseDatedPanels, parseFollowed, wireKind, parseAnswer, rowKey, rowName, rowReason } from './message.ts';
import type { FollowItem, FollowedMessage, FollowedRows } from './message';

const NO_TEXT = 0xffff_ffff;
const encoder = new TextEncoder();
const aligned = (at: number) => (at + 7) & ~7;

/** Rows laid out as an answer's body of kind 1 (`answer.rs`): keys, cells, states, each row's reason and name. */
function rowsBody(rows: number, columns: number, keys: BigUint64Array, values: BigInt64Array, states: Uint8Array,
  reasons: readonly (string | null)[], names: readonly (string | null)[]): Uint8Array {
  const texts: Uint8Array[] = [], index = new Map<string, number>();
  const textOf = (text: string | null) => {
    if (text === null) return NO_TEXT;
    let at = index.get(text);
    if (at === undefined) { at = texts.push(encoder.encode(text)) - 1; index.set(text, at); }
    return at;
  };
  const reasonAt = reasons.map(textOf), nameAt = names.map(textOf);
  const cells = rows * columns;
  const statesAt = 16 + rows * 16 + cells * 8, reasonsAt = aligned(statesAt + cells), namesAt = aligned(reasonsAt + rows * 4);
  let size = aligned(namesAt + rows * 4);
  const textsAt = size;
  for (const text of texts) size += 4 + text.length;
  const out = new Uint8Array(aligned(size)), data = new DataView(out.buffer);
  data.setUint8(0, 1); data.setUint32(4, rows, true); data.setUint32(8, columns, true); data.setUint32(12, texts.length, true);
  new BigUint64Array(out.buffer, 16, rows * 2).set(keys);
  new BigInt64Array(out.buffer, 16 + rows * 16, cells).set(values);
  out.set(states, statesAt);
  new Uint32Array(out.buffer, reasonsAt, rows).set(reasonAt);
  new Uint32Array(out.buffer, namesAt, rows).set(nameAt);
  let at = textsAt;
  for (const text of texts) { data.setUint32(at, text.length, true); out.set(text, at + 4); at += 4 + text.length; }
  return out;
}

interface HeldRows { body: Uint8Array; rows?: FollowedRows; byKey?: Map<bigint, number> }

/** Only the decoder owns the held body. An outgoing snapshot can be transferred while the next delta updates it. */
function overlay(held: HeldRows, changed: FollowedRows): FollowedRows {
  const whole = held.rows ??= parseAnswer(held.body) as FollowedRows;
  if (changed.columns !== whole.columns) throw new Error(`Changed rows of ${changed.columns} columns for an answer of ${whole.columns}`);
  const { rows, columns } = whole, cells = rows * columns;
  if (!held.byKey) {
    held.byKey = new Map();
    for (let row = 0; row < rows; row++) held.byKey.set(rowKey(whole, row), row);
  }
  let reasons: (string | null)[] | undefined, names: (string | null)[] | undefined;
  for (let row = 0; row < changed.rows; row++) {
    const at = held.byKey.get(rowKey(changed, row));
    if (at === undefined) continue;
    for (let column = 0; column < columns; column++) {
      const source = row * columns + column, target = at * columns + column;
      whole.views.i64[whole.values + target] = changed.views.i64[changed.values + source]!;
      whole.views.u8[whole.states + target] = changed.views.u8[changed.states + source]!;
    }
    const reason = rowReason(changed, row), name = rowName(changed, row);
    if (reason !== rowReason(whole, at) || name !== rowName(whole, at) || reasons) {
      reasons ??= Array.from({ length: rows }, (_, i) => rowReason(whole, i));
      names ??= Array.from({ length: rows }, (_, i) => rowName(whole, i));
      reasons[at] = reason;
      names[at] = name;
    }
  }
  if (reasons && names) {
    held.body = rowsBody(rows, columns, whole.views.u64.subarray(whole.keys, whole.keys + rows * 2),
      whole.views.i64.subarray(whole.values, whole.values + cells), whole.views.u8.subarray(whole.states, whole.states + cells), reasons, names);
    held.rows = undefined;
  }
  return parseAnswer(held.body.slice()) as FollowedRows;
}

export type Release = number | { route: number };

export class FollowDecoder {
  private readonly routes = new Map<number, { seek: number; whole: Map<number, HeldRows> }>();

  release(what: Release) {
    if (typeof what === 'number') {
      for (const route of this.routes.values()) route.whole.delete(what);
    } else this.routes.delete(what.route);
  }

  item(buffer: ArrayBuffer): FollowItem {
    const bytes = new Uint8Array(buffer);
    if (wireKind(bytes) === DATED_PANEL) return { kind: 'panels', batch: parseDatedPanels(bytes) };
    const failure = followFailure(bytes);
    if (failure !== null) return { kind: 'failure', reason: failure };
    return { kind: 'followed', message: this.decode(parseFollowed(bytes)) };
  }

  decode(message: FollowedMessage) {
    const seek = message.seek;
    let route = this.routes.get(message.route);
    if (!route || route.seek !== seek || message.reset) {
      route = { seek, whole: new Map() };
      this.routes.set(message.route, route);
    }
    const whole = route.whole;
    message.answers = message.answers.flatMap(entry => {
      const { id, answer, span } = entry;
      if (answer.kind === 'changed') {
        const held = whole.get(id);
        // Not yet read whole: nothing to lay them over (the next whole answer brings them).
        if (!held) return [];
        return [{ id, hour: entry.hour, answer: overlay(held, answer) }];
      }
      if (answer.kind === 'rows' && span) whole.set(id, { body: answer.views.u8.slice(span[0], span[1]) });
      else if (answer.kind === 'notRecorded') whole.delete(id);
      return [entry];
    });
    return message;
  }
}

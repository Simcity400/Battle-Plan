import type { LiveValues } from '../../charts/live';
import { CELL, cellNumber, cellState, cellValue, rowName, rowReason } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer, FollowedRows } from '../../dashboard/follow/message';
import type { TableValues } from './surface';

const NOT_READ: LiveValues<never> = { get: () => undefined, missing: () => 'Not read yet' };

export function namedValues<V>(answer: FollowedAnswer | undefined, read: (answer: FollowedRows, at: number) => V): LiveValues<V> {
  if (!answer) return NOT_READ;
  if (answer.kind === 'notRecorded') { const reason = answer.reason; return { get: () => undefined, missing: () => reason }; }
  const rows = new Map<string, number>();
  for (let row = 0; row < answer.rows; row++) {
    const name = rowName(answer, row);
    if (name !== null && !rows.has(name)) rows.set(name, row);
  }
  return {
    get: name => {
      const row = rows.get(name);
      return row !== undefined && cellState(answer, row * answer.columns) === CELL.value ? read(answer, row * answer.columns) : undefined;
    },
    missing: name => {
      const row = rows.get(name);
      if (row === undefined) return 'Not in the answer';
      return rowReason(answer, row) ?? (cellState(answer, row * answer.columns) === CELL.missing ? 'No value' : 'Not recorded at this hour');
    },
  };
}

const exact = (answer: FollowedRows, at: number): number | bigint => cellNumber(answer, at) ?? cellValue(answer, at);

const decoded = new WeakMap<FollowedRows, LiveValues<number | bigint>>();
function valuesOf(answer: FollowedAnswer | undefined): LiveValues<number | bigint> {
  if (!answer || answer.kind === 'notRecorded') return namedValues(answer, exact);
  let values = decoded.get(answer);
  if (!values) decoded.set(answer, values = namedValues(answer, exact));
  return values;
}

const parts = new Map<string, { name: string; source: string }>();
function partsOf(key: string) {
  let held = parts.get(key);
  if (!held) {
    if (parts.size > 1 << 17) parts.clear();
    const at = key.lastIndexOf('\u0000');
    parts.set(key, held = { name: key.slice(0, at), source: key.slice(at + 1) });
  }
  return held;
}

export function sourceValues(answers: ReadonlyMap<string, FollowedAnswer | undefined>): TableValues {
  const sources = new Map([...answers].map(([name, answer]) => [name, valuesOf(answer)]));
  return {
    get: key => { const { name, source } = partsOf(key); return sources.get(source)?.get(name); },
    missing: key => { const { name, source } = partsOf(key); const values = sources.get(source); return values ? values.missing(name) : 'Not followed'; },
  };
}

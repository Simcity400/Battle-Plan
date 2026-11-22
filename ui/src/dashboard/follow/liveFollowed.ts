import { useMemo, useRef } from 'react';
import type { RefObject } from 'react';
import type { Live, LiveValues } from '../../charts/live';
import { useFollow } from './useFollow.ts';
import type { Followed } from './useFollow';
import { CELL, cellNumber, cellState, cellValue, rowReason } from './message.ts';
import type { FollowedAnswer } from './message';
import type { FollowEntry, FollowQuery, FollowStore } from './store';

export function liveFollowed<T>(entry: FollowEntry, read: (entry: FollowEntry) => T): Live<T> {
  let made: { version: number; value: T } | null = null;
  return {
    read: () => {
      if (made?.version !== entry.version) made = { version: entry.version, value: read(entry) };
      return made.value;
    },
    subscribe: changed => entry.subscribe(changed),
  };
}

const NO_PAINT = () => {};

const noStructure = () => '';

export const answered = (answer: FollowedAnswer | undefined) => answer === undefined ? '' : 'answered';

export function useLiveFollow<T>(store: FollowStore | null, query: FollowQuery | null, target: RefObject<Element | null>,
  read: (entry: FollowEntry) => T, structureOf: (answer: FollowedAnswer | undefined) => string = noStructure): Followed & { live: Live<T> | undefined } {
  const reading = useRef(read);
  reading.current = read;
  const followed = useFollow(store, query, target, NO_PAINT, structureOf);
  const entry = followed.entry;
  const live = useMemo(() => store && entry ? liveFollowed(entry, held => reading.current(held)) : undefined, [store, entry]);
  return { ...followed, live };
}

export function answerValues(answer: FollowedAnswer | undefined, keys: readonly string[], scale = 0, column = 0): LiveValues<number> {
  const rows = new Map(keys.map((key, row) => [key, row]));
  const missing = (key: string) => {
    if (!answer) return 'Not read yet';
    if (answer.kind === 'notRecorded') return answer.reason;
    const row = rows.get(key);
    if (row === undefined || row >= answer.rows) return 'Not in the answer';
    return rowReason(answer, row) ?? (cellState(answer, row * answer.columns + column) === CELL.missing ? 'No value' : 'Not recorded at this hour');
  };
  return {
    get: key => {
      const row = rows.get(key);
      if (!answer || answer.kind === 'notRecorded' || row === undefined || row >= answer.rows) return undefined;
      const at = row * answer.columns + column;
      if (cellState(answer, at) !== CELL.value) return undefined;
      return (cellNumber(answer, at) ?? Number(cellValue(answer, at))) / 10 ** scale;
    },
    missing,
  };
}

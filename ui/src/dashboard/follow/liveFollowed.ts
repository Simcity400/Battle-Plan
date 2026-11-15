/** A followed query as a shared chart's live source: read in the chart's frame, never by a render. */
import { useMemo, useRef } from 'react';
import type { RefObject } from 'react';
import type { Live, LiveValues } from '../../charts/live';
import { useFollow } from './useFollow.ts';
import type { Followed } from './useFollow';
import { CELL, cellNumber, cellState, cellValue, rowReason } from './message.ts';
import type { FollowedAnswer } from './message';
import type { FollowEntry, FollowQuery, FollowStore } from './store';

/** What `read` makes of `entry`, made again only when its answer moves; the chart is woken each time one lands. */
export function liveFollowed<T>(store: Pick<FollowStore, 'watch'>, entry: FollowEntry, read: (entry: FollowEntry) => T): Live<T> {
  let made: { version: number; value: T } | null = null;
  return {
    read: () => {
      if (made?.version !== entry.version) made = { version: entry.version, value: read(entry) };
      return made.value;
    },
    subscribe: changed => store.watch(entry, changed),
  };
}

const NO_PAINT = () => {};

const noStructure = () => '';

/** Whether an answer has landed: the one structure a widget waiting for a follow's first answer renders for. */
export const answered = (answer: FollowedAnswer | undefined) => answer === undefined ? '' : 'answered';

/** `query` followed while `target` is on screen, as a live source of what `read` makes of each answer: the widget
 * renders only when the follow's status or `structureOf` its answer changes, never as values land. `live` is undefined
 * until it is followed. */
export function useLiveFollow<T>(store: FollowStore | null, query: FollowQuery | null, target: RefObject<Element | null>,
  read: (entry: FollowEntry) => T, structureOf: (answer: FollowedAnswer | undefined) => string = noStructure): Followed & { live: Live<T> | undefined } {
  const reading = useRef(read);
  reading.current = read;
  const followed = useFollow(store, query, target, NO_PAINT, structureOf);
  const entry = followed.entry;
  const live = useMemo(() => store && entry ? liveFollowed(store, entry, held => reading.current(held)) : undefined, [store, entry]);
  return { ...followed, live };
}

/** A followed answer's column `column` by the keys its rows were asked for, in order (a series' reads, a table's rows),
 * each as a number of `scale` decimals; a row the answer holds no value for is missing for its reason. */
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

/** The live sources a widget last drew from every follow of a set answered, and what they belong to. */
export interface Handoff<T> { key: readonly unknown[]; lives: readonly Live<T>[] }

type Handed<T> = Pick<Followed, 'status' | 'entry'> & { live: Live<T> | undefined };

const sameKey = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((item, at) => Object.is(item, b[at]));

/** The live sources a set of follows is drawn from: theirs once every one is followed and has answered; while some of a
 * new set still wait for their first answer (none refused, none off), the last answered set's, so the widget stays
 * followed through the change; otherwise none. `key` names what the sources belong to (a store, a timeline). */
export function handoffLives<T>(items: readonly Handed<T>[], ready: boolean, held: Handoff<T> | null, key: readonly unknown[]):
  { lives: (Live<T> | undefined)[]; held: Handoff<T> | null } {
  const none = items.map(() => undefined);
  if (ready && items.every(item => item.status === 'followed' && item.live && item.entry?.answer !== undefined)) {
    const lives = items.map(item => item.live!);
    return { lives, held: { key, lives } };
  }
  const waiting = ready && items.every(item => item.status === 'pending' || item.status === 'followed');
  if (waiting && held && sameKey(held.key, key) && held.lives.length === items.length) return { lives: [...held.lives], held };
  return { lives: none, held: waiting ? held : null };
}

/** `handoffLives` held across renders. */
export function useHandoff<T>(items: readonly Handed<T>[], ready: boolean, ...key: unknown[]): (Live<T> | undefined)[] {
  const held = useRef<Handoff<T> | null>(null);
  const handed = handoffLives(items, ready, held.current, key);
  held.current = handed.held;
  return handed.lives;
}

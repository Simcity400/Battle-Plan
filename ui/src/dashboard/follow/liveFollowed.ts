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


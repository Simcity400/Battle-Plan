import type { Gate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import type { RefObject } from 'react';
import type { LiveDrillParts } from '../../charts/drill/DrillParts';
import { usNumber } from '../../charts/numberFormat.ts';
import { answerValues, useLiveFollow } from '../../dashboard/follow/liveFollowed.ts';
import type { FollowEntry, FollowStore } from '../../dashboard/follow/store';
import type { LiveValues } from '../../charts/live';
import { STATES, SUPPLY_FAMILY, supplyParts, totalDivisions } from './supplyHealthData';
import type { SupplyCounts } from './supplyHealthData';

const NO_VALUES: LiveValues<number> = { get: () => undefined, missing: () => 'Not read yet' };

export function useSupplyHead(store: FollowStore | null, gate: Pick<Gate, 'follow'>, following: boolean, branchId: string | undefined, tag: string | null,
  root: RefObject<Element | null>, colors: readonly string[]): LiveDrillParts | undefined {
  const query = (measure: string) => following && branchId && tag ? gate.follow({ product: 'series-state', params: { branchId,
    family: { kind: SUPPLY_FAMILY, measure }, reads: [{ country: countryFromKey(tag) }] } }) : null;
  const queries = useMemo(() => STATES.map(query), [following, branchId, tag, gate.follow]);
  const keys = useMemo(() => tag ? [tag] : [], [tag]);
  const read = (entry: FollowEntry) => answerValues(entry.answer, keys);
  const none = useLiveFollow(store, queries[0], root, read).live;
  const strained = useLiveFollow(store, queries[1], root, read).live;
  const held = useLiveFollow(store, queries[2], root, read).live;
  return useMemo(() => {
    if (!none || !strained || !held || !tag) return undefined;
    const sources = [none, strained, held];
    let last: { values: readonly LiveValues<number>[]; made: ReturnType<LiveDrillParts['read']> } | null = null;
    return {
      read: () => {
        const values = sources.map(source => source.read() ?? NO_VALUES);
        if (last && last.values.every((value, at) => value === values[at])) return last.made;
        const read = values.map(value => value.get(tag));
        const made = read.every(value => value !== undefined)
          ? (() => {
            const counts = Object.fromEntries(STATES.map((state, at) => [state, read[at]!])) as SupplyCounts;
            const total = totalDivisions(counts);
            return { parts: supplyParts(counts, colors), total, totalText: usNumber.format(total) };
          })()
          : { parts: [], total: null, totalText: values[read.findIndex(value => value === undefined)].missing(tag) };
        last = { values, made };
        return made;
      },
      subscribe: changed => {
        const stops = sources.map(source => source.subscribe(changed));
        return () => stops.forEach(stop => stop());
      },
    };
  }, [none, strained, held, tag, colors]);
}

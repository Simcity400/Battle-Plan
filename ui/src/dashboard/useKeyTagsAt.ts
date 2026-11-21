import { useMemo } from 'react';
import { useAtHead, useIdentityHour } from './atHead.ts';
import { useCountryLabelsAt } from './countryFlagsAt';
import type { CountryLabel } from './countryFlagsAt';
import { isSlotKey } from './countryKey.ts';
import { keyNameAt, useSlotTags } from './countrySlots.ts';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';

const sameTags = (a: readonly (string | null)[], b: readonly (string | null)[]) => a.length === b.length && a.every((tag, i) => tag === b[i]);

export function useKeyTagsAt(runtime: Pick<WidgetRuntime, 'client' | 'clock'>, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, keys: readonly string[],
  shownHour: (clockHour: number) => number, follow: boolean, active: boolean): readonly (string | null)[] {
  const bySlot = keys.some(isSlotKey);
  const slots = useSlotTags(runtime.client, track, active && bySlot);
  return useAtHead(runtime.clock, active && follow && bySlot, clockHour => keys.map(key => isSlotKey(key) ? keyNameAt(slots, key, shownHour(clockHour)) : key), sameTags);
}

export function useKeyLabelsAt(runtime: Pick<WidgetRuntime, 'client' | 'clock'>, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, keys: readonly string[],
  shownHour: (clockHour: number) => number | null, follow: boolean, active: boolean): readonly ReturnType<CountryLabel>[] {
  const hour = useIdentityHour(runtime.client, track, keys, runtime.clock, active && follow && !!track, shownHour);
  const keysKey = JSON.stringify(keys);
  const wanted = useMemo(() => hour === null || !Number.isFinite(hour) ? [] : (JSON.parse(keysKey) as string[]).map(tag => ({ tag, hour })), [keysKey, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && !!track);
  return useMemo(() => wanted.length ? wanted.map(item => labelAt(item.hour)(item.tag)) : [], [wanted, labelAt]);
}

export const chipName = (label: ReturnType<CountryLabel> | undefined) => label?.known === true && !label.nameFailed ? label.name
  : label?.known === undefined && !label?.nameFailed && label !== undefined ? 'Loading…' : 'Country name unavailable';

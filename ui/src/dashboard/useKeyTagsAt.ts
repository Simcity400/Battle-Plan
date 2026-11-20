import { useMemo } from 'react';
import { useAtHead, useIdentityHour } from './atHead.ts';
import { useCountryLabelsAt } from './countryFlagsAt';
import type { CountryLabel } from './countryFlagsAt';
import { isSlotKey } from './countryKey.ts';
import { keyNameAt, useSlotTags } from './countrySlots.ts';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';

const sameTags = (a: readonly (string | null)[], b: readonly (string | null)[]) => a.length === b.length && a.every((tag, i) => tag === b[i]);

/** The tag each [`countryKey`] goes by at the hour shown (`shownHour` of the clock's hour), for a label or flag: a tag
 * key's own tag, a slot's tag then (else the nearest it held); null while that is not known. Read each frame while
 * `follow`ing playback. */
export function useKeyTagsAt(runtime: Pick<WidgetRuntime, 'client' | 'clock'>, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, keys: readonly string[],
  shownHour: (clockHour: number) => number, follow: boolean, active: boolean): readonly (string | null)[] {
  const bySlot = keys.some(isSlotKey);
  const slots = useSlotTags(runtime.client, track, active && bySlot);
  return useAtHead(runtime.clock, active && follow && bySlot, clockHour => keys.map(key => isSlotKey(key) ? keyNameAt(slots, key, shownHour(clockHour)) : key), sameTags);
}

/** Each [`countryKey`]'s label at the hour shown (`shownHour` of the clock's hour): its name and flag as the recording's
 * name resolver gives them then (`useCountryLabelsAt`), the one every country label reads. Following, they are read at
 * the last hour at or before the head where one of them changed (`useIdentityHour`), so they render only when they do. */
export function useKeyLabelsAt(runtime: Pick<WidgetRuntime, 'client' | 'clock'>, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, keys: readonly string[],
  shownHour: (clockHour: number) => number | null, follow: boolean, active: boolean): readonly ReturnType<CountryLabel>[] {
  const hour = useIdentityHour(runtime.client, track, keys, runtime.clock, active && follow && !!track, shownHour);
  const keysKey = JSON.stringify(keys);
  const wanted = useMemo(() => hour === null || !Number.isFinite(hour) ? [] : (JSON.parse(keysKey) as string[]).map(tag => ({ tag, hour })), [keysKey, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && !!track);
  return useMemo(() => wanted.length ? wanted.map(item => labelAt(item.hour)(item.tag)) : [], [wanted, labelAt]);
}

/** A label's name for a chip: the recorded name, else that it is loading or unavailable. */
export const chipName = (label: ReturnType<CountryLabel> | undefined) => label?.known === true && !label.nameFailed ? label.name
  : label?.known === undefined && !label?.nameFailed && label !== undefined ? 'Loading…' : 'Country name unavailable';

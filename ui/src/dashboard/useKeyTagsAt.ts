import { useMemo } from 'react';
import {
  countryKeyFromString,
  flagChoiceKey,
  useIdentityAtHead,
  useIdentityScope,
  type CountryIdentityResult,
} from '../families/identity/react.ts';
import type { CountryLabel } from './countryFlagsAt';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';

export function useKeyTagsAt(
  runtime: Pick<WidgetRuntime, 'client' | 'clock'>,
  track: Pick<BoundTrack, 'source' | 'branch'> | undefined,
  keys: readonly string[],
  shownHour: (clockHour: number) => number,
  follow: boolean,
  active: boolean,
): readonly (string | null)[] {
  const scope = useIdentityScope(runtime.client, track?.source, track?.branch.id, active);
  const keysKey = JSON.stringify(keys);
  const parsed = useMemo(() => (JSON.parse(keysKey) as string[]).map(countryKeyFromString), [keysKey]);
  const branchBounds = useMemo(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] as const : undefined),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const { results } = useIdentityAtHead(runtime.client, track?.source, scope, runtime.clock, parsed, shownHour, {
    active: active && !!track,
    follow,
    branchBounds,
  });
  return useMemo(() => parsed.map((key, index) => {
    if (key.by === 'tag') return key.tag;
    const res = results[index] as CountryIdentityResult | undefined;
    return res?.kind === 'known' && res.tag.kind === 'known' ? res.tag.value : null;
  }), [parsed, results]);
}

export function useKeyLabelsAt(
  runtime: Pick<WidgetRuntime, 'client' | 'clock'>,
  track: Pick<BoundTrack, 'source' | 'branch'> | undefined,
  keys: readonly string[],
  shownHour: (clockHour: number) => number | null,
  follow: boolean,
  active: boolean,
): readonly ReturnType<CountryLabel>[] {
  const scope = useIdentityScope(runtime.client, track?.source, track?.branch.id, active);
  const keysKey = JSON.stringify(keys);
  const parsed = useMemo(() => (JSON.parse(keysKey) as string[]).map(countryKeyFromString), [keysKey]);
  const branchBounds = useMemo(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] as const : undefined),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const { results, flagFor } = useIdentityAtHead(runtime.client, track?.source, scope, runtime.clock, parsed, shownHour, {
    active: active && !!track,
    follow,
    needFlags: true,
    flagSize: 'panel',
    branchBounds,
  });
  return useMemo(() => {
    if (!parsed.length) return [];
    return parsed.map((key, index): ReturnType<CountryLabel> => {
      const res = results[index] as CountryIdentityResult | undefined;
      const fallbackTag = key.by === 'tag' ? key.tag : '';
      if (!res || res.kind === 'pending') {
        return { name: fallbackTag, known: undefined, flagPending: true };
      }
      if (res.kind === 'failed') {
        return {
          name: fallbackTag,
          known: false,
          nameFailed: true,
          nameFailure: res.failure,
          nameDetail: res.failure.message,
          flagPending: false,
          flagDetail: res.failure.message,
        };
      }
      if (res.kind === 'absent') {
        return {
          name: fallbackTag,
          known: false,
          nameDetail: res.reason,
          flagPending: false,
          flagReason: res.reason,
        };
      }
      const tag = res.tag.kind === 'known' ? res.tag.value : fallbackTag;
      const nameKnown = res.name.kind === 'known';
      const namePending = res.name.kind === 'pending';
      const nameFailed = res.name.kind === 'failed';
      const name = nameKnown ? res.name.value : tag;
      const nameDetail = res.name.kind === 'missing' ? res.name.reason : res.name.kind === 'failed' ? res.name.failure.message : undefined;
      let flag: string | undefined;
      let flagPending = false;
      let flagReason: string | undefined;
      let flagDetail: string | undefined;
      if (res.flagChoice.kind === 'known') {
        void flagChoiceKey(res.flagChoice.value);
        const drawn = flagFor(res.flagChoice.value);
        if (!drawn) {
          flagPending = true;
        } else if (drawn.failed) {
          flagDetail = drawn.failed.message;
        } else if (drawn.url) {
          flag = drawn.url;
        } else {
          flagReason = drawn.reason;
        }
      } else if (res.flagChoice.kind === 'pending') {
        flagPending = true;
      } else if (res.flagChoice.kind === 'missing') {
        flagReason = res.flagChoice.reason;
      } else {
        flagDetail = res.flagChoice.failure.message;
      }
      return {
        name,
        known: namePending ? undefined : nameKnown,
        ...(nameFailed && res.name.kind === 'failed' ? { nameFailed: true, nameFailure: res.name.failure } : {}),
        ...(nameDetail ? { nameDetail } : {}),
        ...(flag ? { flag } : {}),
        flagPending,
        ...(flagReason ? { flagReason } : {}),
        ...(flagDetail ? { flagDetail } : {}),
      };
    });
  }, [parsed, results, flagFor]);
}

export const chipName = (label: ReturnType<CountryLabel> | undefined) => label?.known === true && !label.nameFailed ? label.name
  : label?.known === undefined && !label?.nameFailed && label !== undefined ? 'Loading…' : 'Country name unavailable';

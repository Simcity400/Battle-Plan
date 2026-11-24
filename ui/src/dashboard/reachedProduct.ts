import { nameReadMetadata } from './nameReads.ts';
import { useMemo } from 'react';
import type { DashboardClient } from './client';
import { nameKey, useCountryNamesAt } from './countryNamesAt';
import type { BoundTrack } from './model';

export function useCountryNameAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tag: string | null, hour: number | null, active: boolean) {
  const available = !!track && !!tag && hour !== null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const wanted = useMemo(() => available && tag && hour !== null ? [{ hour, tag }] : [], [available, tag, hour]);
  const names = useCountryNamesAt(client, track, wanted, active && available);
  return nameReadMetadata(available && tag && hour !== null ? names.get(nameKey(hour, tag)) : undefined);
}

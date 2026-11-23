import { nameReadMetadata } from './nameReads.ts';
import { useMemo } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import { useContentRevision } from './contentRevision';
import { loadTransientProduct } from './products';
import { nameKey, useCountryNamesAt } from './countryNamesAt';
import type { BoundTrack } from './model';
import { reachedKeys, reachedView, type ReachedState } from './reachedState.ts';
import { useViewRead } from './viewRead';

export type { ReachedState };

export function useReachedProduct<T>(client: DashboardClient, source: DashboardSource | undefined, request: ProductRequest | null,
  subject: string, active: boolean, validate: (data: unknown) => T): ReachedState<T> {
  const content = useContentRevision(client);
  const { owner, key } = reachedKeys(request, subject, content);
  const read = useViewRead<ProductRequest, T>(key === null ? null : owner, request, key ?? '', active,
    (asked, signal) => loadTransientProduct<unknown>(client, asked, signal, source).then(validate), client);
  return { ...reachedView(read, key), retry: read.retry };
}

export function useCountryNameAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tag: string | null, hour: number | null, active: boolean) {
  const available = !!track && !!tag && hour !== null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const wanted = useMemo(() => available && tag && hour !== null ? [{ hour, tag }] : [], [available, tag, hour]);
  const names = useCountryNamesAt(client, track, wanted, active && available);
  return nameReadMetadata(available && tag && hour !== null ? names.get(nameKey(hour, tag)) : undefined);
}

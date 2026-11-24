import { useMemo } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { Failure } from '../../dashboard/queryError.ts';
import { productRequest } from '../../dashboard/request.ts';
import type { StrategicRegionsReply } from '../../generated/dashboard';
import { regionKey, useIdentities, useIdentityScope } from '../identity/react.ts';

export interface RegionListEntry {
  readonly id: number;
  readonly label: string;
  readonly nameMissing: boolean;
  readonly sea: boolean;
}

export type RegionList =
  | { readonly kind: 'loading' }
  | { readonly kind: 'failed'; readonly failure: Failure; readonly retry: () => void }
  | { readonly kind: 'ready'; readonly regions: readonly RegionListEntry[] };

export function useRegionList(client: DashboardClient, track: BoundTrack | undefined, active: boolean): RegionList {
  const request = track
    ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'strategic-regions', params: { branchId: track.branch.id } })
    : null;
  const listed = useProduct<StrategicRegionsReply>(client, request, active, undefined, track?.source);
  const scope = useIdentityScope(client, track?.source, track?.branch.id, active);
  const hour = track?.branch.lastHour ?? 0;
  const asks = useMemo(() => (listed.data?.regions ?? []).map(region => ({ key: regionKey(region.id), hour })), [listed.data, hour]);
  const names = useIdentities(client, track?.source, active ? scope : null, asks, { active });
  return useMemo((): RegionList => {
    if (listed.error) return { kind: 'failed', failure: listed.error, retry: listed.retry };
    if (!listed.data) return { kind: 'loading' };
    const regions = listed.data.regions.map((region): RegionListEntry => {
      const named = names.resolve(regionKey(region.id), hour);
      const name = named.kind === 'known' && named.name.kind === 'known' ? named.name.value : null;
      const reason = named.kind === 'known' && named.name.kind === 'missing' ? named.name.reason
        : named.kind === 'absent' ? named.reason
          : named.kind === 'pending' || (named.kind === 'known' && named.name.kind === 'pending') ? 'Loading name…'
            : 'Region name not recorded';
      return { id: region.id, label: name ?? `${reason} (${region.id})`, nameMissing: name === null, sea: region.sea };
    });
    regions.sort((a, b) => a.label.localeCompare(b.label) || a.id - b.id);
    return { kind: 'ready', regions };
  }, [listed.data, listed.error, listed.retry, names, hour]);
}

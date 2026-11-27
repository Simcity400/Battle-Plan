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
  readonly sea: boolean | null;
  readonly membershipMissing: string | null;
}

export type RegionList =
  | { readonly kind: 'loading' }
  | { readonly kind: 'failed'; readonly failure: Failure; readonly retry: () => void }
  | { readonly kind: 'ready'; readonly regions: readonly RegionListEntry[] };

const CLOSED: RegionList = { kind: 'loading' };
const NO_ASKS: readonly { readonly key: ReturnType<typeof regionKey>; readonly hour: number }[] = [];

export function useRegionList(client: DashboardClient, track: BoundTrack | undefined, active: boolean, hour: number | null): RegionList {
  const open = active && !!track;
  const request = useMemo(
    () => (open && track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'strategic-regions', params: { branchId: track.branch.id } }) : null),
    [open, track?.source.id, track?.source.revision, track?.branch.id],
  );
  const listed = useProduct<StrategicRegionsReply>(client, request, open, undefined, track?.source);
  const scope = useIdentityScope(client, track?.source, track?.branch.id, open);
  const regions = open ? listed.data?.regions : undefined;
  const asks = useMemo(() => (regions && hour !== null ? regions.map(region => ({ key: regionKey(region.id), hour })) : NO_ASKS), [regions, hour]);
  const names = useIdentities(client, track?.source, open ? scope : null, asks, { active: open });
  return useMemo((): RegionList => {
    if (!open) return CLOSED;
    if (listed.error) return { kind: 'failed', failure: listed.error, retry: listed.retry };
    if (!regions) return CLOSED;
    const entries = regions.map((region): RegionListEntry => {
      const membership = { sea: region.sea, membershipMissing: region.membershipMissing };
      if (hour === null) return { id: region.id, label: `Region ${region.id}`, nameMissing: true, ...membership };
      const named = names.resolve(regionKey(region.id), hour);
      const name = named.kind === 'known' && named.name.kind === 'known' ? named.name.value : null;
      const reason = named.kind === 'known' && named.name.kind === 'missing' ? named.name.reason
        : named.kind === 'absent' ? named.reason
          : named.kind === 'pending' || (named.kind === 'known' && named.name.kind === 'pending') ? 'Loading name…'
            : 'Region name not recorded';
      return { id: region.id, label: name ?? `${reason} (${region.id})`, nameMissing: name === null, ...membership };
    });
    entries.sort((a, b) => a.label.localeCompare(b.label) || a.id - b.id);
    return { kind: 'ready', regions: entries };
  }, [open, regions, listed.error, listed.retry, names.version, hour, names.resolve]);
}

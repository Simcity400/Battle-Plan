import { useMemo } from 'react';
import type { DashboardClient } from '../dashboard/client.ts';
import type { BoundTrack } from '../dashboard/model.ts';
import { useIdentity, useIdentityScope } from '../families/identity/react.ts';
import { provinceKey } from '../families/identity/types.ts';
import { NAME_LOADING } from '../families/table/identityCell.ts';
import { knownValue, notRecorded, pendingValue, type Value } from '../families/value.ts';

export function useBattleTitle(client: DashboardClient, track: BoundTrack, province: number | null, hour: number): Value<string> {
  const scope = useIdentityScope(client, track.source, track.branch.id);
  const key = useMemo(() => province === null ? null : provinceKey(province), [province]);
  const { result } = useIdentity(client, track.source, scope, key, hour, { branchBounds: [track.branch.firstHour, track.branch.lastHour] });
  if (province === null) return notRecorded('Province not recorded');
  const name = result?.kind === 'known' ? result.name : result;
  if (!name || name.kind === 'pending') return pendingValue(NAME_LOADING);
  if (name.kind === 'known') return name.value.trim() ? knownValue(`Battle of ${name.value}`) : notRecorded('Province name not recorded');
  return notRecorded(name.kind === 'failed' ? name.failure.message : name.reason);
}

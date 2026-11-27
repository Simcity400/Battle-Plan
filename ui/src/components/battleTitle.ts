import { useMemo } from 'react';
import type { DashboardClient } from '../dashboard/client.ts';
import type { BoundTrack } from '../dashboard/model.ts';
import { useIdentity, useIdentityScope } from '../families/identity/react.ts';
import { battleKey } from '../families/identity/types.ts';
import { LOADING_NAME, NAME_LOADING } from '../families/table/identityCell.ts';
import { knownValue, notRecorded, pendingValue, type Value } from '../families/value.ts';

export function landBattleTitleKey(key: string): string {
  return `battle:${key.slice(key.indexOf(':') + 1)}`;
}

export function battleTitleText(title: Value<string>): string {
  return title.kind === 'known' ? title.value : title.kind === 'pending' ? LOADING_NAME : 'Battle name not recorded';
}

export function useBattleTitle(client: DashboardClient, track: BoundTrack, battle: string, hour: number): Value<string> {
  const scope = useIdentityScope(client, track.source, track.branch.id);
  const key = useMemo(() => battleKey(battle), [battle]);
  const { result } = useIdentity(client, track.source, scope, key, hour, { branchBounds: [track.branch.firstHour, track.branch.lastHour] });
  const name = result?.kind === 'known' ? result.name : result;
  if (!name || name.kind === 'pending') return pendingValue(NAME_LOADING);
  if (name.kind === 'known') return knownValue(name.value);
  return notRecorded(name.kind === 'failed' ? name.failure.message : name.reason);
}

import { useMemo } from 'react';
import type { DashboardClient } from '../dashboard/client.ts';
import type { BoundTrack } from '../dashboard/model.ts';
import { useIdentity, useIdentityScope } from '../families/identity/react.ts';
import { battleKey } from '../families/identity/types.ts';
import { NAME_LOADING } from '../families/table/identityCell.ts';
import { ValueState } from './ValueState.tsx';

export function BattleTitleHeading({ client, track, battle }: { client: DashboardClient; track: BoundTrack; battle: string }) {
  const scope = useIdentityScope(client, track.source, track.branch.id);
  const key = useMemo(() => battleKey(battle), [battle]);
  const { firstHour, lastHour } = track.branch;
  const { result } = useIdentity(client, track.source, scope, key, firstHour, { branchBounds: [firstHour, lastHour] });
  const name = result?.kind === 'known' ? result.name : result;
  if (!name || name.kind === 'pending') return <ValueState kind="pending" reason={NAME_LOADING} />;
  if (name.kind === 'known') return name.value;
  return <ValueState kind="missing" reason={name.kind === 'failed' ? name.failure.message : name.reason} />;
}

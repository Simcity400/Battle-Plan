import { useCallback, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { EntityKind } from '../contract.ts';
import { parseEntityKey } from '../identity/provider.ts';
import { useIdentityScope, useIdentitySnapshot, type IdentityAsk, type IdentitySnapshot } from '../identity/react.ts';

export function useTimelineIdentity(
  client: DashboardClient,
  track: WidgetRuntime['tracks'][number] | undefined,
  active: boolean,
): {
  readonly snapshot: IdentitySnapshot | null;
  readonly onDemands: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
} {
  const [asks, setAsks] = useState<readonly IdentityAsk[]>([]);
  const sigRef = useRef('');
  const onDemands = useCallback((demands: readonly (readonly [EntityKind, string, number])[]) => {
    const sig = demands.map(([k, key, h]) => `${k}\u0000${key}\u0000${h}`).join('\u0001');
    if (sig === sigRef.current) return;
    sigRef.current = sig;
    setAsks(demands.map(([k, key, h]) => ({ key: parseEntityKey(k, key), hour: h })));
  }, []);

  const scope = useIdentityScope(client, track?.source, track?.branch.id, active && !!track);
  const branchBounds = useMemo(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] as const : undefined),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const snapshot = useIdentitySnapshot(client, track?.source, scope, asks, {
    active: active && !!track,
    needFlags: true,
    flagSize: 'panel',
    branchBounds,
  });

  return { snapshot, onDemands };
}

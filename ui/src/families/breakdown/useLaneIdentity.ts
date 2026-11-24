import { useCallback, useMemo, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { parseEntityKey } from '../identity/provider.ts';
import { useIdentityScope, useIdentitySnapshot, type IdentityAsk, type IdentitySnapshot } from '../identity/react.ts';
import type { EntityKind } from '../plan.ts';

export function useLaneIdentity(
  client: DashboardClient,
  track: BoundTrack | undefined,
  active: boolean,
  extraAsks?: readonly IdentityAsk[],
  options?: { readonly needFlags?: boolean; readonly needGroups?: boolean },
): {
  readonly identitySnapshot: IdentitySnapshot | undefined;
  readonly onIdentityDemands: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
} {
  const [demands, setDemands] = useState<readonly (readonly [EntityKind, string, number])[]>([]);
  const onIdentityDemands = useCallback((next: readonly (readonly [EntityKind, string, number])[]) => {
    setDemands(prev => {
      if (prev.length === next.length && prev.every((item, i) => item[0] === next[i]![0] && item[1] === next[i]![1] && item[2] === next[i]![2])) {
        return prev;
      }
      return next;
    });
  }, []);

  const scope = useIdentityScope(client, track?.source, track?.branch.id, active);
  const asks = useMemo<readonly IdentityAsk[]>(() => {
    const out: IdentityAsk[] = extraAsks ? [...extraAsks] : [];
    for (const [kind, key, hour] of demands) {
      out.push({ key: parseEntityKey(kind, key), hour });
    }
    return out;
  }, [extraAsks, demands]);

  const bounds = useMemo<readonly [number, number] | undefined>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : undefined,
    [track?.branch.firstHour, track?.branch.lastHour],
  );

  const snapshot = useIdentitySnapshot(client, track?.source, scope, asks, {
    active,
    needFlags: options?.needFlags ?? true,
    needGroups: options?.needGroups ?? false,
    branchBounds: bounds,
  });

  return {
    identitySnapshot: snapshot ?? undefined,
    onIdentityDemands,
  };
}

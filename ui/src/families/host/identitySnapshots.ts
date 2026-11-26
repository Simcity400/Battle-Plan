import type { DashboardSource } from '../../dashboard/client';
import type { IdentityBroker } from '../identity/broker.ts';
import type { HourInterval, IdentityScope, IdentitySnapshot } from '../identity/types.ts';

export interface IdentitySnapshots {
  readonly subscribe: (changed: () => void) => () => void;
  readonly read: () => IdentitySnapshot | null;
}

const NONE: IdentitySnapshots = { subscribe: () => () => {}, read: () => null };

export function identitySnapshots(
  broker: IdentityBroker,
  scope: IdentityScope | null,
  recording: DashboardSource | undefined,
  branchBounds: HourInterval | undefined,
): IdentitySnapshots {
  if (!scope) return NONE;
  return {
    subscribe: changed => broker.subscribe(scope, changed, recording, branchBounds),
    read: () => broker.snapshotOf(scope, recording, branchBounds),
  };
}

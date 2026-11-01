import type { DashboardMapState } from '../../dashboard/client';
import type { MapTarget } from '../../dashboard/geometry';

/** Whether the acknowledged frame answers the target. Paused, it must be the
 * target's hour at its sequence. Playing, the clock moves on every hour, so any
 * ready frame of the same branch answers it, including a native clock between host updates. */
export function mapPresentationMatches(state: DashboardMapState | null, desired: MapTarget | null): boolean {
  if (!desired || !state || state.loading || state.error || state.pending || state.hour == null) return false;
  if (state.sourceId !== desired.sourceId || state.revision !== desired.revision || state.branchId !== desired.branchId) return false;
  return desired.playing || state.hour === desired.hour && state.sequence >= desired.sequence;
}

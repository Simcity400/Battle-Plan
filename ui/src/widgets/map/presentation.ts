import type { DashboardMapState } from '../../dashboard/client';
import type { MapTarget } from '../../dashboard/geometry';

export function mapPresentationMatches(state: DashboardMapState | null, desired: MapTarget | null): boolean {
  if (!desired || !state || state.loading || state.error || state.hour == null) return false;
  if (state.sourceId !== desired.sourceId || state.revision !== desired.revision || state.branchId !== desired.branchId) return false;
  return desired.playing || state.hour === desired.hour && state.sequence >= desired.sequence;
}

export function mapFrameOnScreen(state: DashboardMapState | null): boolean {
  return !!state && !state.error && state.hour != null && state.branchId != null;
}

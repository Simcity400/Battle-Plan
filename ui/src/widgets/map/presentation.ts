import type { DashboardMapState } from '../../dashboard/client';
import type { MapTarget } from '../../dashboard/geometry';
import type { PlaybackController } from '../../dashboard/playback';

export function mapPresentationMatches(state: DashboardMapState | null, desired: MapTarget | null): boolean {
  if (!desired || !state || state.loading || state.error || state.hour == null) return false;
  if (state.sourceId !== desired.sourceId || state.revision !== desired.revision || state.branchId !== desired.branchId) return false;
  return desired.playing || state.hour === desired.hour && state.sequence >= desired.sequence;
}

export function mapFrameOnScreen(state: DashboardMapState | null): boolean {
  return !!state && !state.error && state.hour != null && state.branchId != null;
}

export function mapRequests(clock: Pick<PlaybackController, 'issue'>): (target: Omit<MapTarget, 'sequence'>) => MapTarget {
  let last = { key: '', sequence: 0 };
  return (target) => {
    const key = JSON.stringify([target.sourceId, target.path, target.revision, target.branchId, !!target.playing, target.seekSequence ?? null, target.playing ? null : target.hour]);
    if (key !== last.key) last = { key, sequence: clock.issue() };
    return { ...target, sequence: last.sequence };
  };
}

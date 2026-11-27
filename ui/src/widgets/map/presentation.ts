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
    const battle = target.battle;
    const key = JSON.stringify([
      target.sourceId, target.path, target.revision, target.branchId, !!target.playing, target.seekSequence ?? null, target.hour,
      target.focusProvince ?? null,
      battle ? [battle.kind ?? null, battle.combat ?? null, battle.result ? [battle.result.handle, battle.result.from] : null, battle.country, battle.countrySlot ?? null, battle.live ?? null] : null,
    ]);
    if (key !== last.key) last = { key, sequence: clock.issue() };
    return { ...target, sequence: last.sequence };
  };
}

import { resolveWindow, type BoundTrack, type HourWindow, type WidgetSpec } from './model.ts';
import type { RecordingGap } from '../generated/dashboard';

declare const TRACK: unique symbol;
declare const AXIS: unique symbol;
export type TrackHour = number & { readonly [TRACK]: 'track' };
export type AxisHour = number & { readonly [AXIS]: 'axis' };

export type RegionAction = { kind: 'spec' } | { kind: 'follow' } | { kind: 'at'; hour: TrackHour | null };
export interface Region { widget: WidgetSpec; action: RegionAction }
export interface Outside { message: string; detail: string | undefined }
export interface RegionTime { hour: TrackHour | null; window: HourWindow | null; follows: boolean; outside: Outside | null }

export const OUTSIDE_MESSAGE = 'This time is outside the recording.';
export const UNRECORDED_MESSAGE = 'This time was not recorded.';
export const SETTINGS_DETAIL = 'Choose another time in widget settings.';
export const SPEC: RegionAction = { kind: 'spec' };
export const FOLLOW: RegionAction = { kind: 'follow' };

export const axisHour = (clock: { getHour(): number }): AxisHour => clock.getHour() as AxisHour;
export const axisAt = (position: number): AxisHour => position as AxisHour;

export type CoveredBranch = Pick<BoundTrack['branch'], 'firstHour' | 'lastHour' | 'gaps'>;
type Branch = { branch: CoveredBranch };

interface GapIndex { gaps: readonly RecordingGap[]; reach: readonly number[] }
const gapIndexes = new WeakMap<readonly RecordingGap[], GapIndex>();

function gapIndex(gaps: readonly RecordingGap[]): GapIndex {
  let index = gapIndexes.get(gaps);
  if (index) return index;
  const sorted = [...gaps].sort((a, b) => a.firstHour - b.firstHour);
  const reach: number[] = [];
  for (const gap of sorted) reach.push(Math.max(reach.at(-1) ?? -Infinity, gap.lastHour));
  gapIndexes.set(gaps, index = { gaps: sorted, reach });
  return index;
}

export function gapAt(track: Branch, hour: number): RecordingGap | null {
  const gaps = track.branch.gaps;
  if (!gaps?.length) return null;
  const { gaps: sorted, reach } = gapIndex(gaps);
  let low = 0, high = sorted.length;
  while (low < high) { const middle = (low + high) >> 1; if (sorted[middle].firstHour <= hour) low = middle + 1; else high = middle; }
  for (let at = low - 1; at >= 0 && reach[at] >= hour; at--) if (sorted[at].lastHour >= hour) return sorted[at];
  return null;
}

export function coverageAt(track: Branch | undefined, hour: number | null | undefined): { hour: TrackHour } | { hour: null; outside: Outside } {
  if (!track || hour === null || hour === undefined || !Number.isFinite(hour)) return { hour: null, outside: { message: OUTSIDE_MESSAGE, detail: undefined } };
  const at = Math.floor(hour);
  if (at < track.branch.firstHour || at > track.branch.lastHour) return { hour: null, outside: { message: OUTSIDE_MESSAGE, detail: undefined } };
  const gap = gapAt(track, at);
  return gap ? { hour: null, outside: { message: UNRECORDED_MESSAGE, detail: gap.reason } } : { hour: at as TrackHour };
}

export function recordedHour(track: Branch | undefined, hour: number | null | undefined): TrackHour | null {
  return coverageAt(track, hour).hour;
}

export function axisHourOf(track: Pick<BoundTrack, 'axisOffset'>, hour: TrackHour): AxisHour {
  return hour - track.axisOffset as AxisHour;
}

export const at = (hour: TrackHour | null): RegionAction => ({ kind: 'at', hour });

export function pickedAt(track: Branch | undefined, hour: number | null | undefined): RegionAction {
  return at(recordedHour(track, hour));
}

export function regionFollows(region: Region): boolean {
  return region.action.kind === 'follow' || region.action.kind === 'spec' && region.widget.time.mode === 'follow';
}

const followed = (widget: WidgetSpec): WidgetSpec => widget.time.mode === 'follow' ? widget : { ...widget, time: { mode: 'follow' } };

export function regionWindow(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[], axis: AxisHour): HourWindow | null {
  if (!track) return null;
  const { widget, action } = region;
  if (action.kind === 'spec') return resolveWindow(widget, track, tracks as BoundTrack[], axis);
  if (action.kind === 'follow') return resolveWindow(followed(widget), track, tracks as BoundTrack[], axis);
  return action.hour === null ? null : resolveWindow(followed(widget), track, tracks as BoundTrack[], axisHourOf(track, action.hour));
}

export function regionHour(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[], axis: AxisHour): TrackHour | null {
  return recordedHour(track, regionWindow(region, track, tracks, axis)?.lastHour);
}

const ownTime = (region: Region) => region.action.kind === 'spec' && region.widget.time.mode !== 'follow';

export function regionOutside(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[], axis: AxisHour): Outside | null {
  if (!track) return null;
  const covered = coverageAt(track, regionWindow(region, track, tracks, axis)?.lastHour);
  if (covered.hour !== null) return null;
  if (!ownTime(region)) return covered.outside;
  const { message, detail } = covered.outside;
  return { message, detail: detail ? `${detail} ${SETTINGS_DETAIL}` : SETTINGS_DETAIL };
}

export function resolveRegion(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[], axis: AxisHour): RegionTime {
  const window = regionWindow(region, track, tracks, axis);
  const hour = recordedHour(track, window?.lastHour);
  return { hour, window: hour === null ? null : window, follows: regionFollows(region), outside: hour === null ? regionOutside(region, track, tracks, axis) : null };
}

export function regionKey(region: Region): string {
  const { widget, action } = region;
  const time = action.kind === 'at' ? `at:${action.hour}` : regionFollows(region) ? 'follow' : JSON.stringify(widget.time);
  return `${widget.trackIds[0] ?? ''}\u0000${time}`;
}

export function regionTrack(region: Region, tracks: readonly BoundTrack[]): BoundTrack | undefined {
  return tracks.find(track => track.id === region.widget.trackIds[0]);
}

export type StageState =
  | { stage: 'outside'; outside: Outside }
  | { stage: 'absent' }
  | { stage: 'no-members' }
  | { stage: 'loading' }
  | { stage: 'ready' };

export interface Stages { hour: TrackHour | null; outside?: Outside | null; country?: 'absent' | 'loading' | 'held' | null; members?: 'none' | 'loading' | 'held' | null; read?: 'loading' | 'held' | null }

export function stageOf({ hour, outside = null, country = null, members = null, read = null }: Stages): StageState {
  if (hour === null) return { stage: 'outside', outside: outside ?? { message: OUTSIDE_MESSAGE, detail: undefined } };
  if (country === 'loading') return { stage: 'loading' };
  if (country === 'absent') return { stage: 'absent' };
  if (members === 'loading') return { stage: 'loading' };
  if (members === 'none') return { stage: 'no-members' };
  if (read === 'loading') return { stage: 'loading' };
  return { stage: 'ready' };
}

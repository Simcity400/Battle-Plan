import { resolveWindow, type BoundTrack, type HourWindow, type WidgetSpec } from './model.ts';

declare const TRACK: unique symbol;
declare const AXIS: unique symbol;
export type TrackHour = number & { readonly [TRACK]: 'track' };
export type AxisHour = number & { readonly [AXIS]: 'axis' };

export type RegionAction = { kind: 'spec' } | { kind: 'follow' } | { kind: 'at'; hour: TrackHour | null };
export interface Region { widget: WidgetSpec; action: RegionAction }
export interface RegionTime { hour: TrackHour | null; window: HourWindow | null; follows: boolean; detail: string | undefined }

export const SETTINGS_DETAIL = 'Choose another time in widget settings.';
export const SPEC: RegionAction = { kind: 'spec' };
export const FOLLOW: RegionAction = { kind: 'follow' };

export const axisHour = (clock: { getHour(): number }): AxisHour => clock.getHour() as AxisHour;
export const axisAt = (position: number): AxisHour => position as AxisHour;

type Branch = Pick<BoundTrack, 'branch'>;

export function recordedHour(track: Branch | undefined, hour: number | null | undefined): TrackHour | null {
  if (!track || hour === null || hour === undefined || !Number.isFinite(hour)) return null;
  const at = Math.floor(hour);
  return at < track.branch.firstHour || at > track.branch.lastHour ? null : at as TrackHour;
}

export function trackHourOn(track: Pick<BoundTrack, 'axisOffset'>, axis: AxisHour): number {
  return Math.floor(axis) + track.axisOffset;
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

export function regionClockHour(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[], axis: AxisHour): number | null {
  const last = regionWindow(region, track, tracks, axis)?.lastHour;
  return last === undefined ? null : Math.floor(last);
}

export function regionDetail(region: Region, hour: TrackHour | null): string | undefined {
  return hour === null && region.action.kind === 'spec' && region.widget.time.mode !== 'follow' ? SETTINGS_DETAIL : undefined;
}

export function resolveRegion(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[], axis: AxisHour): RegionTime {
  const window = regionWindow(region, track, tracks, axis);
  const hour = recordedHour(track, window?.lastHour);
  return { hour, window: hour === null ? null : window, follows: regionFollows(region), detail: regionDetail(region, hour) };
}

export function regionKey(region: Region): string {
  const { widget, action } = region;
  const time = action.kind === 'at' ? `at:${action.hour}` : regionFollows(region) ? 'follow' : JSON.stringify(widget.time);
  return `${widget.trackIds[0] ?? ''}\u0000${time}`;
}

export function regionTrack(region: Region, tracks: readonly BoundTrack[]): BoundTrack | undefined {
  return tracks.find(track => track.id === region.widget.trackIds[0]);
}

export type DrillTime = { follow: boolean; hour: number | null };

export function drillAction(drill: DrillTime, track: Branch | undefined): RegionAction {
  return drill.follow ? FOLLOW : pickedAt(track, drill.hour);
}

export type StageState =
  | { stage: 'outside'; detail: string | undefined }
  | { stage: 'absent' }
  | { stage: 'no-members' }
  | { stage: 'loading' }
  | { stage: 'ready' };

export interface Stages { hour: TrackHour | null; detail?: string; country?: 'absent' | 'loading' | 'held' | null; members?: 'none' | 'loading' | 'held' | null; read?: 'loading' | 'held' | null }

export function stageOf({ hour, detail, country = null, members = null, read = null }: Stages): StageState {
  if (hour === null) return { stage: 'outside', detail };
  if (country === 'loading') return { stage: 'loading' };
  if (country === 'absent') return { stage: 'absent' };
  if (members === 'loading') return { stage: 'loading' };
  if (members === 'none') return { stage: 'no-members' };
  if (read === 'loading') return { stage: 'loading' };
  return { stage: 'ready' };
}

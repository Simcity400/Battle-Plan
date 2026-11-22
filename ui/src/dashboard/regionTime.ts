import { resolveWindow, type BoundTrack, type HourWindow, type WidgetSpec } from './model.ts';

import type { RecordingGap } from '../generated/dashboard';

declare const TRACK: unique symbol;
declare const AXIS: unique symbol;
export type TrackHour = number & { readonly [TRACK]: 'track' };
export type AxisHour = number & { readonly [AXIS]: 'axis' };
declare const BOUND: unique symbol;
export type BoundHour = TrackHour & { readonly [BOUND]: 'bound' };
declare const GATED: unique symbol;
export type GatedHour = BoundHour & { readonly [GATED]: 'gated' };
export interface BoundWindow { firstHour: BoundHour; lastHour: BoundHour }
export interface GatedWindow { firstHour: BoundHour; lastHour: GatedHour }
type Offsets = { firstOffset: number; lastOffset: number };
export type PinTime = { mode: 'follow' } | { mode: 'pin'; windows: Record<string, Offsets> } | { mode: 'range' } & Offsets;
let mintTime: (time: PinTime) => GatedTime;
export class GatedTime {
  readonly #time: PinTime;
  private constructor(time: PinTime) { this.#time = time; }
  static { mintTime = time => new GatedTime(time); }
  get value(): PinTime { return this.#time; }
  toJSON(): PinTime { return this.#time; }
}

type Timed = Pick<BoundTrack, 'id'> & { branch: CoveredBranch };
const heldOffsets = (track: Timed | undefined, { firstOffset, lastOffset }: Offsets): Offsets | null =>
  track && Number.isSafeInteger(firstOffset) && Number.isSafeInteger(lastOffset) && 0 <= firstOffset && firstOffset <= lastOffset
    && coverageAt(track, track.branch.firstHour + lastOffset).hour !== null ? { firstOffset, lastOffset } : null;

export function validTime(time: PinTime, tracks: readonly Timed[], rangeTrack?: string): GatedTime | null {
  if (time.mode === 'follow') return mintTime({ mode: 'follow' });
  if (time.mode === 'range') {
    const held = heldOffsets(tracks.find(track => track.id === rangeTrack), time);
    return held && mintTime({ mode: 'range', ...held });
  }
  const windows: Record<string, Offsets> = {};
  for (const [id, offsets] of Object.entries(time.windows)) {
    const held = heldOffsets(tracks.find(track => track.id === id), offsets);
    if (!held) return null;
    windows[id] = held;
  }
  return Object.keys(windows).length ? mintTime({ mode: 'pin', windows }) : null;
}

export type PinnedSpec = Omit<WidgetSpec, 'time'> & { readonly time: GatedTime };
export const persistedSpec = (pinned: PinnedSpec): WidgetSpec => ({ ...pinned, time: pinned.time.value });

declare const FOLLOWED: unique symbol;
export type FollowQuery = Readonly<Record<string, unknown>> & { readonly [FOLLOWED]: 'gated' };
export type GatedParams = {
  readonly [key: string]: unknown;
  readonly [key: `${string}Hour`]: BoundHour | null | undefined;
  readonly hour?: GatedHour | null;
  readonly from?: BoundHour | null;
  readonly to?: BoundHour | null;
  readonly reads?: readonly ({ readonly hour: GatedHour } | GatedHour)[];
  readonly hours?: readonly GatedHour[];
};

export type HourRole = 'exact' | 'bound';
const roleOf = (key: string): HourRole | null =>
  key === 'hour' || key === 'hours' || key === 'reads' ? 'exact' : key === 'from' || key === 'to' || key.endsWith('Hour') ? 'bound' : null;

export function hourFields(value: unknown, visit: (role: HourRole, hour: unknown, path: string) => void, path = 'params'): void {
  if (Array.isArray(value)) { value.forEach((item, index) => { if (item !== null && typeof item === 'object') hourFields(item, visit, `${path}[${index}]`); }); return; }
  if (value === null || typeof value !== 'object') return;
  for (const [key, item] of Object.entries(value)) {
    const role = roleOf(key), at = `${path}.${key}`;
    if (role && Array.isArray(item)) item.forEach((each, index) => { if (each === null || typeof each !== 'object') visit(role, each, `${at}[${index}]`); });
    else if (role && item !== null && item !== undefined && typeof item !== 'object' && !(key === 'reads' && typeof item === 'string')) visit(role, item, at);
    hourFields(item, visit, at);
  }
}

export function refusedHours(params: unknown, branch: CoveredBranch, followed = false): string | null {
  let refused: string | null = null;
  hourFields(params, (role, hour, path) => {
    if (refused !== null) return;
    if (role === 'exact' && followed) refused = `${path}: a followed query reads at playback and names no hour`;
    else if (typeof hour !== 'number' || !Number.isSafeInteger(hour)) refused = `${path} is not a whole hour`;
    else if (hour < branch.firstHour || hour > branch.lastHour) refused = `${path} names hour ${hour}, outside branch ${branch.firstHour}–${branch.lastHour}`;
    else if (role === 'exact' && gapAt({ branch }, hour)) refused = `${path} names hour ${hour}, which was not recorded`;
  });
  return refused;
}

export type RegionAction = { kind: 'spec' } | { kind: 'follow' } | { kind: 'at'; hour: number };
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

export function coverageSpan(track: Branch, hour: number): readonly [number, number] {
  const at = Math.floor(hour), { firstHour, lastHour } = track.branch;
  if (at < firstHour) return [-Infinity, firstHour - 1];
  if (at > lastHour) return [lastHour + 1, Infinity];
  const gaps = track.branch.gaps;
  if (!gaps?.length) return [firstHour, lastHour];
  const gap = gapAt(track, at);
  if (gap) return [Math.max(firstHour, gap.firstHour), Math.min(lastHour, gap.lastHour)];
  const { gaps: sorted, reach } = gapIndex(gaps);
  let low = 0, high = sorted.length;
  while (low < high) { const middle = (low + high) >> 1; if (sorted[middle].firstHour <= at) low = middle + 1; else high = middle; }
  return [Math.max(firstHour, low ? reach[low - 1] + 1 : firstHour), Math.min(lastHour, low < sorted.length ? sorted[low].firstHour - 1 : lastHour)];
}

export function recordedHour(track: Branch | undefined, hour: number | null | undefined): TrackHour | null {
  return coverageAt(track, hour).hour;
}

export const at = (hour: number): RegionAction => ({ kind: 'at', hour: Math.floor(hour) });

export function regionFollows(region: Region): boolean {
  return region.action.kind === 'follow' || region.action.kind === 'spec' && region.widget.time.mode === 'follow';
}

const followed = (widget: WidgetSpec): WidgetSpec => widget.time.mode === 'follow' ? widget : { ...widget, time: { mode: 'follow' } };

export function regionWindow(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[], axis: AxisHour): HourWindow | null {
  if (!track) return null;
  const { widget, action } = region;
  if (action.kind === 'spec') return resolveWindow(widget, track, tracks as BoundTrack[], axis);
  if (action.kind === 'follow') return resolveWindow(followed(widget), track, tracks as BoundTrack[], axis);
  return Number.isFinite(action.hour) ? resolveWindow(followed(widget), track, tracks as BoundTrack[], action.hour - track.axisOffset) : null;
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

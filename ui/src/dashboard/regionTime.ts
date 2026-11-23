import { resolveWindow, type BoundTrack, type HourWindow, type WidgetSpec } from './model.ts';
import { formatDateTime, gameDate } from './calendar.ts';

import type { AnyRequestParams, HourRole, RecordingGap, RequestParams } from '../generated/dashboard';
import { REQUEST_HOURS, REQUEST_PATHS } from '../generated/dashboard.ts';

declare const TRACK: unique symbol;
declare const AXIS: unique symbol;
export type TrackHour = number & { readonly [TRACK]: 'track' };
export type AxisHour = number & { readonly [AXIS]: 'axis' };
declare const BOUND: unique symbol;
export type BoundHour = TrackHour & { readonly [BOUND]: 'bound' };
declare const GATED: unique symbol;
export type GatedHour = BoundHour & { readonly [GATED]: 'gated' };
export type BoundWindow = { firstHour: BoundHour; lastHour: BoundHour };
export type GatedWindow = { firstHour: BoundHour; lastHour: GatedHour };
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
  track && Number.isSafeInteger(firstOffset) && Number.isSafeInteger(lastOffset) && firstOffset <= lastOffset
    && coverageAt(track, track.branch.firstHour + lastOffset).hour !== null ? { firstOffset: Math.max(0, firstOffset), lastOffset } : null;

export type TimeRefusal = { time: GatedTime; refused?: undefined } | { time: null; refused: string };

export function checkedTime(time: PinTime, tracks: readonly Timed[], rangeTrack?: string): TimeRefusal {
  if (time.mode === 'follow') return { time: mintTime({ mode: 'follow' }) };
  if (time.mode === 'range') {
    const held = heldOffsets(tracks.find(track => track.id === rangeTrack), time);
    return held ? { time: mintTime({ mode: 'range', ...held }) } : { time: null, refused: 'The range’s last hour was not recorded.' };
  }
  const windows: Record<string, Offsets> = {};
  for (const [id, offsets] of Object.entries(time.windows)) {
    const held = heldOffsets(tracks.find(track => track.id === id), offsets);
    if (held) windows[id] = held;
  }
  return Object.keys(windows).length ? { time: mintTime({ mode: 'pin', windows }) }
    : { time: null, refused: 'No track recorded this moment, so there is nothing to pin.' };
}

export const validTime = (time: PinTime, tracks: readonly Timed[], rangeTrack?: string): GatedTime | null => checkedTime(time, tracks, rangeTrack).time;

export type PinnedSpec = Omit<WidgetSpec, 'time'> & { readonly time: GatedTime };
export const persistedSpec = (pinned: PinnedSpec): WidgetSpec => ({ ...pinned, time: pinned.time.value });

declare const FOLLOWED: unique symbol;
export type FollowQuery = Readonly<Record<string, unknown>> & { readonly [FOLLOWED]: 'gated' };
type Unchecked = { readonly [key: string]: unknown };
export type ParamsFor<K extends string> = Unchecked & AnyRequestParams<GatedHour, BoundHour>
  & (K extends keyof RequestParams<GatedHour, BoundHour> ? RequestParams<GatedHour, BoundHour>[K] : unknown);
export type GatedParams = ParamsFor<string>;

export type { HourRole };
export const exactRole = (role: HourRole): boolean => role === 'read' || role === 'bucket';

export function requestFields(product: string): Readonly<Record<string, HourRole>> | null {
  return Object.hasOwn(REQUEST_HOURS, product) ? REQUEST_HOURS[product] : null;
}

export function followedProduct(query: unknown): { product: string; params: unknown } | null {
  if (query === null || typeof query !== 'object') return null;
  const asked = query as Readonly<Record<string, unknown>>;
  if (typeof asked.product === 'string') return { product: asked.product, params: asked.params };
  if (typeof asked.panel === 'string') { const { panel, ...params } = asked; return { product: panel, params }; }
  if (typeof asked.head === 'string') return { product: `head:${asked.head}`, params: asked.params };
  return null;
}

const plainObject = (value: unknown): value is Readonly<Record<string, unknown>> => value !== null && typeof value === 'object' && !Array.isArray(value);
type Refuse = (path: string, expected: string) => void;

function visitPath(value: unknown, segments: readonly string[], path: string, visit: (hour: unknown, path: string) => void, refuse: Refuse): void {
  if (!segments.length) { if (value !== null && value !== undefined) visit(value, path); return; }
  if (value === null || value === undefined) return;
  const [head, ...rest] = segments;
  const many = head.endsWith('[]'), key = many ? head.slice(0, -2) : head, index = /^\d+$/.test(key);
  if (typeof value !== 'object') return;
  if (index ? !Array.isArray(value) : !plainObject(value)) { refuse(path, index ? 'a list' : 'an object naming its fields'); return; }
  const item = index ? (value as readonly unknown[])[Number(key)] : (value as Readonly<Record<string, unknown>>)[key], at = index ? `${path}[${key}]` : `${path}.${key}`;
  if (!many) { visitPath(item, rest, at, visit, refuse); return; }
  if (item === null || item === undefined) return;
  if (typeof item !== 'object') return;
  if (!Array.isArray(item)) { refuse(at, 'a list'); return; }
  item.forEach((each, position) => visitPath(each, rest, `${at}[${position}]`, visit, refuse));
}

function declaredNames(value: unknown, at: string, paths: Readonly<Record<string, string>>, shown: string, refuse: Refuse): void {
  if (Array.isArray(value)) {
    const items = `${at}[]`;
    value.forEach((item, position) => {
      const path = Object.hasOwn(paths, items) ? items : `${at}.${position}`;
      if (Object.hasOwn(paths, path)) declaredNames(item, path, paths, `${shown}[${position}]`, refuse);
    });
    return;
  }
  if (!plainObject(value) || (at !== '' && paths[at] !== 'node')) return;
  const wild = at ? `${at}.*` : '*';
  for (const [key, item] of Object.entries(value)) {
    const path = Object.hasOwn(paths, wild) ? wild : at ? `${at}.${key}` : key;
    if (!Object.hasOwn(paths, path)) refuse(`${shown}.${key}`, 'a field the request type has');
    else declaredNames(item, path, paths, `${shown}.${key}`, refuse);
  }
}

export function hourFields(product: string, params: unknown, visit: (role: HourRole, hour: unknown, path: string) => void, refuse: Refuse = () => {}): boolean {
  const fields = requestFields(product);
  if (!fields) return false;
  if (!plainObject(params)) { refuse('params', 'an object naming its fields'); return true; }
  if (Object.hasOwn(REQUEST_PATHS, product)) declaredNames(params, '', REQUEST_PATHS[product], 'params', refuse);
  for (const [path, role] of Object.entries(fields)) visitPath(params, path.split('.'), 'params', (hour, at) => visit(role, hour, at), refuse);
  return true;
}

export function refusedHours(product: string, params: unknown, branch: CoveredBranch, followed = false): string | null {
  let refused: string | null = null;
  const declared = hourFields(product, params, (role, hour, path) => {
    if (refused !== null) return;
    const exact = exactRole(role);
    if (exact && followed) refused = `${path}: a followed query reads at playback and names no hour`;
    else if (typeof hour !== 'number' || !Number.isSafeInteger(hour)) refused = `${path} is not a whole hour`;
    else if (hour < branch.firstHour || hour > branch.lastHour) refused = `${path} names hour ${hour}, outside branch ${branch.firstHour}–${branch.lastHour}`;
    else if (exact && gapAt({ branch }, hour)) refused = `${path} names hour ${hour}, which was not recorded`;
  }, (path, expected) => { refused ??= `${path} is not ${expected}, as the contract declares`; });
  return declared ? refused : `${product} is not a declared product`;
}

export type RegionAction = { kind: 'spec' } | { kind: 'follow' } | { kind: 'at'; hour: number };
export interface Region { widget: WidgetSpec; action: RegionAction }
export interface Outside {
  kind?: 'outside' | 'not-recorded';
  message: string;
  detail: string | undefined;
  span?: { firstHour: number; lastHour: number };
  pinned?: boolean;
}
export interface RegionTime { hour: TrackHour | null; window: HourWindow | null; follows: boolean; outside: Outside | null }

export const OUTSIDE_MESSAGE = 'This time is outside the recording.';
export const UNRECORDED_MESSAGE = 'Not recorded at this time.';
export const SETTINGS_DETAIL = 'Choose another time in widget settings.';
export const SPEC: RegionAction = { kind: 'spec' };
export const FOLLOW: RegionAction = { kind: 'follow' };

export const axisHour = (clock: { getHour(): number }): AxisHour => clock.getHour() as AxisHour;
export const axisAt = (position: number): AxisHour => position as AxisHour;

export type CoveredBranch = Pick<BoundTrack['branch'], 'firstHour' | 'lastHour' | 'gaps'>;
type Branch = { branch: CoveredBranch };

interface GapIndex { gaps: readonly RecordingGap[]; reach: readonly number[]; edges: readonly number[] }
const gapIndexes = new WeakMap<readonly RecordingGap[], GapIndex>();

function gapIndex(gaps: readonly RecordingGap[]): GapIndex {
  let index = gapIndexes.get(gaps);
  if (index) return index;
  const sorted = [...gaps].sort((a, b) => a.firstHour - b.firstHour);
  const reach: number[] = [];
  for (const gap of sorted) reach.push(Math.max(reach.at(-1) ?? -Infinity, gap.lastHour));
  const edges = [...new Set(sorted.flatMap(gap => [gap.firstHour, gap.lastHour + 1]))].sort((a, b) => a - b);
  gapIndexes.set(gaps, index = { gaps: sorted, reach, edges });
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

function gapDetail(gap: RecordingGap): string {
  return gap.reason?.trim() ?? '';
}

export function coverageAt(track: Branch | undefined, hour: number | null | undefined): { hour: TrackHour } | { hour: null; outside: Outside } {
  const boundsDetail = track && gameDate(track.branch.firstHour) && gameDate(track.branch.lastHour)
    ? `The recording covers ${formatDateTime(track.branch.firstHour)} to ${formatDateTime(track.branch.lastHour)}.`
    : undefined;
  const boundsSpan = track ? { firstHour: track.branch.firstHour, lastHour: track.branch.lastHour } : undefined;
  if (!track || hour === null || hour === undefined || !Number.isFinite(hour)) {
    return { hour: null, outside: { kind: 'outside', message: OUTSIDE_MESSAGE, detail: boundsDetail, span: boundsSpan } };
  }
  const at = Math.floor(hour);
  if (at < track.branch.firstHour || at > track.branch.lastHour) {
    return { hour: null, outside: { kind: 'outside', message: OUTSIDE_MESSAGE, detail: boundsDetail, span: boundsSpan } };
  }
  const gap = gapAt(track, at);
  return gap
    ? { hour: null, outside: { kind: 'not-recorded', message: UNRECORDED_MESSAGE, detail: gapDetail(gap) || undefined, span: { firstHour: gap.firstHour, lastHour: gap.lastHour } } }
    : { hour: at as TrackHour };
}

export function coverageSpan(track: Branch, hour: number): readonly [number, number] {
  const at = Math.floor(hour), { firstHour, lastHour } = track.branch;
  if (at < firstHour) return [-Infinity, firstHour - 1];
  if (at > lastHour) return [lastHour + 1, Infinity];
  const gaps = track.branch.gaps;
  if (!gaps?.length) return [firstHour, lastHour];
  const { edges } = gapIndex(gaps);
  let low = 0, high = edges.length;
  while (low < high) { const middle = (low + high) >> 1; if (edges[middle] <= at) low = middle + 1; else high = middle; }
  return [Math.max(firstHour, low ? edges[low - 1] : firstHour), Math.min(lastHour, low < edges.length ? edges[low] - 1 : lastHour)];
}

export function recordedHour(track: Branch | undefined, hour: number | null | undefined): TrackHour | null {
  return coverageAt(track, hour).hour;
}

export function bucketEdges(track: Branch, size: number): readonly number[] {
  const edges: number[] = [];
  for (let edge = (Math.floor(track.branch.firstHour / size) + 1) * size; edge <= track.branch.lastHour; edge += size) edges.push(edge);
  return edges;
}

export function bucketRead(track: Branch | undefined, hour: number | null, size: number, within?: readonly HourWindow[] | null): TrackHour | null {
  if (!track || hour === null) return null;
  const start = Math.max(track.branch.firstHour, Math.floor(hour / size) * size);
  const last = Math.min(track.branch.lastHour, Math.floor(hour / size) * size + size - 1);
  const pieces = (within ?? [{ firstHour: start, lastHour: last }])
    .map(span => ({ firstHour: Math.max(start, span.firstHour), lastHour: Math.min(last, span.lastHour) }))
    .filter(span => span.firstHour <= span.lastHour).sort((a, b) => a.firstHour - b.firstHour);
  for (const piece of pieces) for (let at = piece.firstHour; at <= piece.lastHour; at = coverageSpan(track, at)[1] + 1) {
    const recorded = recordedHour(track, at);
    if (recorded !== null) return recorded;
  }
  return null;
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
  return { ...covered.outside, pinned: true };
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

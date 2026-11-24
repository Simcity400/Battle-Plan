import { newId } from '../shell/ids.ts';
import type { DashboardSource } from './client';
import { widgetDefinition } from './definitions.ts';
import { resolveOptions } from './options.ts';
import type { WidgetOptions } from './options';
import { pageDefinition } from './pages.ts';
import type { PageId } from './pages.ts';

export const TRAILING_HOURS = 168;
export const WIDGET_CONTENT_SCALES = [50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200] as const;
export const GRID_COLUMNS = { lg: 12, md: 12, sm: 6, xs: 2 } as const;
export type Breakpoint = keyof typeof GRID_COLUMNS;
export type Alignment = 'calendar' | 'anchors';
export type WidgetKind = string;
export interface HourWindow { firstHour: number; lastHour: number }
export interface OffsetWindow { firstOffset: number; lastOffset: number }
export type CountryGroup = { type: 'humans' } | { type: 'faction'; id: string } | { type: 'war'; tag: string } | { type: 'top'; count: number };
export type GroupMembership = 'hour' | 'ever';
export type CountrySelection = { kind: 'selected' } | { kind: 'all' } | { kind: 'tags'; tags: string[] }
  | { kind: 'group'; group: CountryGroup; membership: GroupMembership; aggregate: boolean }
  | { kind: 'factions'; count: number; membership: GroupMembership };
export type WidgetTime = { mode: 'follow' } | { mode: 'campaign' }
  | ({ mode: 'range' } & OffsetWindow)
  | { mode: 'pin'; windows: Record<string, OffsetWindow> };
export interface SourceRole { id: string; label: string; selectedTag: string | null; selectedSlot?: number | null; observed?: Record<string, number> }
export interface TrackSpec {
  id: string;
  roleId: string;
  label: string;
  branchIndex: number;
  anchorOffset: number | null;
}
export interface WidgetSpec {
  campaign?: 'default' | 'tracks';
  id: string;
  kind: WidgetKind;
  configVersion: number;
  title: string;
  contentScale?: number;
  trackIds: string[];
  countries: CountrySelection;
  time: WidgetTime;
  options?: WidgetOptions;
}
export type WidgetBase = Omit<WidgetSpec, 'options'>;
export interface GridItem { i: string; x: number; y: number; w: number; h: number; minW?: number; minH?: number }
export type GridLayouts = Partial<Record<Breakpoint, GridItem[]>>;
export interface DashboardTemplate {
  version: 1;
  name: string;
  alignment: Alignment;
  roles: SourceRole[];
  tracks: TrackSpec[];
  widgets: WidgetSpec[];
  layouts: GridLayouts;
}
export interface BoundTrack extends TrackSpec {
  source: DashboardSource;
  branch: DashboardSource['metadata']['branches'][number];
  axisOffset: number;
  selectedTag: string | null;
  selectedSlot?: number | null;
  observed?: Record<string, number>;
}

export function createWidget(kind: WidgetKind, trackId: string, id: string = newId()): WidgetSpec {
  const definition = widgetDefinition(kind);
  if (!definition) throw new Error(`Unknown widget type: ${kind}`);
  return {
    id, kind, configVersion: definition.version, title: definition.title, campaign: 'default',
    trackIds: [trackId], countries: definition.defaultCountries ?? (definition.countries?.all ? { kind: 'all' } : { kind: 'selected' }),
    time: { mode: definition.defaultTime }, contentScale: 100,
  };
}

export function widgetAtDefault(widget: WidgetSpec, trackId: string): boolean {
  const fields = widgetDefinition(widget.kind)?.options;
  const shown = (spec: WidgetSpec) => ({ configVersion: spec.configVersion, title: spec.title, contentScale: spec.contentScale ?? 100, countries: spec.countries, time: spec.time,
    campaign: spec.campaign ?? 'default', trackIds: spec.campaign === 'tracks' ? spec.trackIds : null, options: resolveOptions(fields, spec.options) });
  return stableJson(shown(widget)) === stableJson(shown(createWidget(widget.kind, trackId, widget.id)));
}

export function defaultTemplate(page: PageId = 'overview'): DashboardTemplate {
  const panels = pageDefinition(page)?.panels ?? [];
  const minSize = (kind: WidgetKind) => widgetDefinition(kind)?.minSize ?? { w: 1, h: 4 };
  const widgets = panels.map(([kind, , , , , variant]) => {
    const preset = variant === undefined ? undefined : widgetDefinition(kind)?.variants?.find(item => item.key === variant);
    return { ...createWidget(kind, 'main', `${page}-${kind}${variant === undefined ? '' : `-${variant.replace(/[^\w-]/g, '-')}`}`), ...preset?.initial };
  });
  const wide = panels.map(([kind, x, y, w, h], index) => ({ i: widgets[index].id, x, y, w, h, minW: minSize(kind).w, minH: minSize(kind).h }));
  const narrow = (cols: number) => {
    let y = 0;
    return panels.map(([kind, , , , h], index) => {
      const item = { i: widgets[index].id, x: 0, y, w: cols, h, minW: Math.min(cols, minSize(kind).w), minH: minSize(kind).h };
      y += h;
      return item;
    });
  };
  return {
    version: 1, name: 'Default layout', alignment: 'calendar',
    roles: [{ id: 'primary', label: 'Primary recording', selectedTag: null }],
    tracks: [{ id: 'main', roleId: 'primary', label: 'Main campaign', branchIndex: 0, anchorOffset: null }],
    widgets, layouts: { lg: wide, md: wide, sm: narrow(6), xs: narrow(2) },
  };
}

function chosenBranch(source: DashboardSource, id: string | undefined) {
  if (id === undefined) return undefined;
  const branches = source.metadata.branches;
  return branches.find(branch => branch.id === id) ?? branches.find(branch => branch.joins.some(join => join.segmentId === id));
}

export function bindTracks(template: DashboardTemplate, sources: ReadonlyMap<string, DashboardSource>, chosen: Readonly<Record<string, string>> = {}): BoundTrack[] {
  return template.tracks.flatMap((track) => {
    const source = sources.get(track.roleId);
    const branch = source && (chosenBranch(source, chosen[track.id]) ?? source.metadata.branches[track.branchIndex]);
    if (!source || !branch) return [];
    if (template.alignment === 'anchors' && track.anchorOffset == null) return [];
    const anchor = branch.firstHour + (track.anchorOffset ?? 0);
    if (template.alignment === 'anchors' && (anchor < branch.firstHour || anchor > branch.lastHour)) return [];
    const role = template.roles.find((item) => item.id === track.roleId);
    return [{ ...track, source, branch, axisOffset: template.alignment === 'calendar' ? 0 : anchor,
      selectedTag: role?.selectedTag ?? null, selectedSlot: role?.selectedSlot ?? null, observed: role?.observed }];
  });
}

export function clockBounds(tracks: BoundTrack[]): HourWindow | null {
  if (!tracks.length) return null;
  return {
    firstHour: Math.min(...tracks.map((track) => track.branch.firstHour - track.axisOffset)),
    lastHour: Math.max(...tracks.map((track) => track.branch.lastHour - track.axisOffset)),
  };
}

export function widgetTracks(widget: WidgetBase, tracks: BoundTrack[]): BoundTrack[] {
  return widget.trackIds.flatMap((id) => {
    const track = tracks.find((item) => item.id === id);
    return track ? [track] : [];
  });
}

export function lastRecordedBranchHour(
  branch: { readonly firstHour: number; readonly lastHour: number; readonly gaps?: readonly { readonly firstHour: number; readonly lastHour: number }[] },
): number | null {
  const { firstHour, lastHour, gaps } = branch;
  if (!Number.isSafeInteger(firstHour) || !Number.isSafeInteger(lastHour) || firstHour > lastHour) return null;
  if (!gaps?.length) return lastHour;
  let at = lastHour;
  while (at >= firstHour) {
    let hit: { readonly firstHour: number; readonly lastHour: number } | null = null;
    for (let i = 0; i < gaps.length; i++) {
      const gap = gaps[i]!;
      if (gap.firstHour <= at && at <= gap.lastHour) {
        if (!hit || gap.firstHour < hit.firstHour) hit = gap;
      }
    }
    if (!hit) return at;
    at = hit.firstHour - 1;
  }
  return null;
}

export function resolveWindow(widget: WidgetBase, track: BoundTrack, tracks: BoundTrack[], position: number): HourWindow | null {
  const time = widget.time;
  if (time.mode === 'campaign') {
    const end = lastRecordedBranchHour(track.branch) ?? track.branch.lastHour;
    return { firstHour: widgetDefinition(widget.kind)?.scope === 'moment' ? end : track.branch.firstHour, lastHour: end };
  }
  if (time.mode === 'pin') {
    const window = time.windows[track.id];
    return window ? { firstHour: track.branch.firstHour + window.firstOffset, lastHour: track.branch.firstHour + window.lastOffset } : null;
  }
  if (time.mode === 'range') {
    const reference = tracks.find((item) => item.id === widget.trackIds[0]);
    if (!reference) return null;
    const origin = reference.branch.firstHour - reference.axisOffset + track.axisOffset;
    return { firstHour: origin + time.firstOffset, lastHour: origin + time.lastOffset };
  }
  const lastHour = Math.floor(position) + track.axisOffset;
  return { firstHour: widgetDefinition(widget.kind)?.scope === 'history' ? lastHour - TRAILING_HOURS + 1 : lastHour, lastHour };
}

export function recordingEndTime(meta: { timeModes: readonly WidgetTime['mode'][]; scope: 'moment' | 'history' },
  tracks: readonly { id: string; branch: HourWindow & { gaps?: readonly { firstHour: number; lastHour: number }[] } }[]): WidgetTime | null {
  if (meta.timeModes.includes('campaign')) return { mode: 'campaign' };
  if (!meta.timeModes.includes('pin')) return null;
  const windows: Record<string, OffsetWindow> = {};
  for (const track of tracks) {
    const endHour = lastRecordedBranchHour(track.branch) ?? track.branch.lastHour;
    const end = endHour - track.branch.firstHour;
    windows[track.id] = { firstOffset: meta.scope === 'history' ? 0 : end, lastOffset: end };
  }
  return { mode: 'pin', windows };
}

export function migrateWidget(widget: WidgetSpec): WidgetSpec {
  const definition = widgetDefinition(widget.kind);
  if (!definition?.migrate || widget.configVersion >= definition.version) return widget;
  const migrated = definition.migrate(widget.configVersion, widget);
  return migrated && migrated.id === widget.id && migrated.kind === widget.kind ? { ...migrated, configVersion: definition.version } : widget;
}

export function rebaseWidget(current: WidgetSpec, base: WidgetSpec, next: WidgetSpec): WidgetSpec {
  const merged: WidgetSpec = { ...current };
  const keys = new Set([...Object.keys(base), ...Object.keys(next)]) as Set<keyof WidgetSpec>;
  for (const key of keys) {
    if (key === 'options' || stableJson(base[key]) === stableJson(next[key])) continue;
    if (Object.hasOwn(next, key)) Object.assign(merged, { [key]: next[key] }); else delete merged[key];
  }
  const options: Record<string, NonNullable<WidgetSpec['options']>[string]> = { ...current.options };
  for (const key of new Set([...Object.keys(base.options ?? {}), ...Object.keys(next.options ?? {})])) {
    if (stableJson(base.options?.[key]) === stableJson(next.options?.[key])) continue;
    if (next.options && Object.hasOwn(next.options, key)) options[key] = next.options[key]; else delete options[key];
  }
  if (Object.keys(options).length) merged.options = options; else delete merged.options;
  return merged;
}

export function mergeViewPreferences(saved: WidgetSpec, previous: WidgetSpec, next: WidgetSpec, savedTracks: ReadonlySet<string>): WidgetSpec {
  const changed = (key: keyof WidgetSpec) => stableJson(previous[key]) !== stableJson(next[key]);
  const merged: WidgetSpec = { ...saved };
  if (changed('title')) merged.title = next.title;
  if (changed('contentScale')) merged.contentScale = next.contentScale;
  if (changed('configVersion')) merged.configVersion = next.configVersion;
  if (changed('options')) {
    const options = { ...saved.options };
    for (const key of new Set([...Object.keys(previous.options ?? {}), ...Object.keys(next.options ?? {})])) {
      if (stableJson(previous.options?.[key]) === stableJson(next.options?.[key])) continue;
      if (next.options && Object.hasOwn(next.options, key)) options[key] = next.options[key]; else delete options[key];
    }
    merged.options = options;
  }
  const time = next.time;
  const tracksKnown = next.trackIds.every(id => savedTracks.has(id))
    && (time.mode !== 'pin' || Object.keys(time.windows).every(id => savedTracks.has(id)));
  if (tracksKnown) {
    const campaign = changed('campaign') || changed('trackIds');
    if (campaign) Object.assign(merged, { campaign: next.campaign, trackIds: next.trackIds });
    if (campaign || changed('countries')) merged.countries = next.countries;
    if (campaign || changed('time')) merged.time = time;
  }
  if (!merged.options || !Object.keys(merged.options).length) delete merged.options;
  return merged;
}

export function stableJson(value: unknown): string {
  return JSON.stringify(value, (_key, item) => item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) : item);
}

export function addGridItem(layouts: GridLayouts, widget: WidgetSpec): GridLayouts {
  const definition = widgetDefinition(widget.kind);
  return Object.fromEntries(Object.entries(GRID_COLUMNS).map(([breakpoint, cols]) => {
    const current = layouts[breakpoint as Breakpoint] ?? [];
    return [breakpoint, [...current, { i: widget.id, x: 0, y: Math.max(0, ...current.map((item) => item.y + item.h)),
      w: Math.min(cols, definition?.defaultSize.w ?? cols), h: definition?.defaultSize.h ?? 8,
      minW: Math.min(cols, definition?.minSize.w ?? 1), minH: definition?.minSize.h ?? 4 }]];
  }));
}

export function addGridItemBelow(layouts: GridLayouts, widget: WidgetSpec, sourceId: string): GridLayouts {
  const definition = widgetDefinition(widget.kind);
  const present = Object.entries(GRID_COLUMNS).filter(([breakpoint]) => layouts[breakpoint as Breakpoint]);
  return Object.fromEntries((present.length ? present : Object.entries(GRID_COLUMNS)).map(([breakpoint, cols]) => {
    const current = layouts[breakpoint as Breakpoint] ?? [];
    const source = current.find(item => item.i === sourceId);
    const w = source ? source.w : Math.min(cols, definition?.defaultSize.w ?? cols);
    const h = definition?.defaultSize.h ?? 8;
    const y = source ? source.y + source.h : Math.max(0, ...current.map(item => item.y + item.h));
    const shifted = current.map(item => item.y >= y ? { ...item, y: item.y + h } : item);
    return [breakpoint, [...shifted, { i: widget.id, x: source?.x ?? 0, y, w, h,
      minW: Math.min(cols, definition?.minSize.w ?? 1), minH: definition?.minSize.h ?? 4 }]];
  }));
}

export function seekPositionFor(track: Pick<BoundTrack, 'axisOffset' | 'branch'>, rawHour: number): number | null {
  if (!Number.isFinite(rawHour) || rawHour < track.branch.firstHour || rawHour > track.branch.lastHour) return null;
  return Math.floor(rawHour) - track.axisOffset;
}


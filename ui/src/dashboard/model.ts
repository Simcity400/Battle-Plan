import { newId } from '../shell/ids.ts';
import type { DashboardSource } from './client';
import { widgetDefinition } from './definitions.ts';

export const TRAILING_HOURS = 168;
export const WIDGET_CONTENT_SCALES = [50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200] as const;
export const GRID_COLUMNS = { lg: 12, md: 12, sm: 6, xs: 2 } as const;
export type Breakpoint = keyof typeof GRID_COLUMNS;
export type Alignment = 'calendar' | 'anchors';
export type WidgetKind = string;
export interface HourWindow { firstHour: number; lastHour: number }
export interface OffsetWindow { firstOffset: number; lastOffset: number }
export type CountrySelection = { kind: 'selected' } | { kind: 'all' } | { kind: 'tags'; tags: string[] };
export type WidgetTime = { mode: 'follow' } | { mode: 'campaign' }
  | ({ mode: 'range' } & OffsetWindow)
  | { mode: 'pin'; windows: Record<string, OffsetWindow> };
export interface SourceRole { id: string; label: string; selectedTag: string | null }
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
}
export interface GridItem { i: string; x: number; y: number; w: number; h: number; minW?: number; minH?: number }
export type GridLayouts = Partial<Record<Breakpoint, GridItem[]>>;
/** Templates deliberately contain no paths, runtime source IDs or query data. */
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
}
export interface Inspection { trackId: string; sourceId: string; revision: string; branchId: string; countryId: string; hour: number }

export function createWidget(kind: WidgetKind, trackId: string, id: string = newId()): WidgetSpec {
  const definition = widgetDefinition(kind);
  if (!definition) throw new Error(`Unknown widget type: ${kind}`);
  return {
    id, kind, configVersion: definition.version, title: definition.title, campaign: 'default',
    trackIds: [trackId], countries: { kind: 'selected' },
    time: { mode: definition.defaultTime }, contentScale: 100,
  };
}

export function defaultTemplate(): DashboardTemplate {
  const panels = [
    ['summary-1', 'Industry', 0, 0, 3, 3],
    ['summary-2', 'Military', 3, 0, 3, 3],
    ['summary-3', 'Research', 6, 0, 3, 3],
    ['summary-4', 'Resources', 9, 0, 3, 3],
    ['map', 'Campaign map', 0, 3, 7, 12],
    ['panel-1', 'National focus', 7, 3, 5, 12],
    ['panel-2', 'Military balance', 7, 15, 5, 7],
    ['panel-3', 'Country comparison', 0, 15, 7, 7],
    ['panel-4', 'Campaign notes', 7, 22, 5, 7],
  ] as const;
  const kindOf = (id: string) => id === 'map' ? 'map' : id === 'panel-1' ? 'national-focus' : 'placeholder';
  const minSize = (id: string) => widgetDefinition(kindOf(id))?.minSize ?? { w: 1, h: 4 };
  const widgets = panels.map(([id, title]) => ({ ...createWidget(kindOf(id), 'main', id), title }));
  const wide = panels.map(([i, , x, y, w, h]) => ({ i, x, y, w, h, minW: minSize(i).w, minH: minSize(i).h }));
  const narrow = (cols: number) => {
    let y = 0;
    return panels.map(([i]) => {
      const h = i === 'map' ? 11 : i.startsWith('summary') ? 3 : 6;
      const item = { i, x: 0, y, w: cols, h, minW: Math.min(cols, minSize(i).w), minH: minSize(i).h };
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

export function branchSelections(tracks: TrackSpec[], selected: Readonly<Record<string, number>>): TrackSpec[] {
  return tracks.map(track => ({ ...track, branchIndex: selected[track.id] ?? track.branchIndex }));
}

export function bindTracks(template: DashboardTemplate, sources: ReadonlyMap<string, DashboardSource>): BoundTrack[] {
  return template.tracks.flatMap((track) => {
    const source = sources.get(track.roleId);
    const branch = source?.metadata.branches[track.branchIndex];
    if (!source || !branch) return [];
    if (template.alignment === 'anchors' && track.anchorOffset == null) return [];
    const anchor = branch.firstHour + (track.anchorOffset ?? 0);
    if (template.alignment === 'anchors' && (anchor < branch.firstHour || anchor > branch.lastHour)) return [];
    return [{ ...track, source, branch, axisOffset: template.alignment === 'calendar' ? 0 : anchor,
      selectedTag: template.roles.find((role) => role.id === track.roleId)?.selectedTag ?? null }];
  });
}

export function clockBounds(tracks: BoundTrack[]): HourWindow | null {
  if (!tracks.length) return null;
  return {
    firstHour: Math.min(...tracks.map((track) => track.branch.firstHour - track.axisOffset)),
    lastHour: Math.max(...tracks.map((track) => track.branch.lastHour - track.axisOffset)),
  };
}

export function widgetTracks(widget: WidgetSpec, tracks: BoundTrack[]): BoundTrack[] {
  return widget.trackIds.flatMap((id) => {
    const track = tracks.find((item) => item.id === id);
    return track ? [track] : [];
  });
}

export function resolveWindow(widget: WidgetSpec, track: BoundTrack, tracks: BoundTrack[], position: number): HourWindow | null {
  const time = widget.time;
  if (time.mode === 'campaign') return { firstHour: widgetDefinition(widget.kind)?.scope === 'moment' ? track.branch.lastHour : track.branch.firstHour, lastHour: track.branch.lastHour };
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

export function pinTime(widget: WidgetSpec, tracks: BoundTrack[], position: number): WidgetTime {
  const windows: Record<string, OffsetWindow> = {};
  for (const track of widgetTracks(widget, tracks)) {
    const window = resolveWindow(widget, track, tracks, position);
    if (window) windows[track.id] = { firstOffset: window.firstHour - track.branch.firstHour, lastOffset: window.lastHour - track.branch.firstHour };
  }
  return { mode: 'pin', windows };
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

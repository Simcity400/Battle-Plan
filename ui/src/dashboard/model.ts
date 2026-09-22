import { newId } from '../ids.ts';
import type { CountryMetadata, CountrySeries, CountrySeriesPoint } from '../api';
import type { MetricKey } from '../analytics/model';
import type { DashboardQuery, DashboardSource } from './client';
import { widgetDefinition, widgetUnavailable } from './definitions.ts';

export const TRAILING_HOURS = 168;
export const PREFETCH_HOURS = 96;
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
  metric: MetricKey;
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
export interface SeriesLayer {
  id: string;
  label: string;
  track: BoundTrack;
  series: CountrySeries;
  window: HourWindow;
}
export interface Inspection { trackId: string; sourceId: string; revision: string; branchId: string; countryId: string; hour: number }

export function createWidget(kind: WidgetKind, trackId: string, id: string = newId()): WidgetSpec {
  const definition = widgetDefinition(kind);
  if (!definition) throw new Error(`Unknown widget type: ${kind}`);
  return {
    id, kind, configVersion: definition.version, title: definition.title, campaign: 'default',
    trackIds: [trackId], countries: { kind: 'selected' }, metric: 'politicalPower',
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
  const widgets = panels.map(([id, title]) => ({ ...createWidget(id === 'map' ? 'map' : id === 'panel-1' ? 'national-focus' : 'placeholder', 'main', id), title }));
  const wide = panels.map(([i, , x, y, w, h]) => ({ i, x, y, w, h, minW: i === 'map' ? 3 : 2, minH: i === 'map' ? 8 : 3 }));
  const narrow = (cols: number) => {
    let y = 0;
    return panels.map(([i]) => {
      const h = i === 'map' ? 11 : i.startsWith('summary') ? 3 : 6;
      const item = { i, x: 0, y, w: cols, h, minW: Math.min(cols, 2), minH: i === 'map' ? 8 : 3 };
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

export function selectCountries(track: BoundTrack, selection: CountrySelection): { countries: CountryMetadata[]; missing: string[] } {
  const catalogue = new Map<string, CountryMetadata>();
  for (const country of track.source.metadata.countries) {
    if (!catalogue.has(country.id) || country.observedBranchId === track.branch.id) catalogue.set(country.id, country);
  }
  const countries = [...catalogue.values()];
  if (selection.kind === 'all') return { countries, missing: countries.length ? [] : ['No observed country identities'] };
  if (selection.kind === 'selected' && track.selectedTag == null) {
    const country = countries.find((item) => item.id === track.source.metadata.playerCountryId);
    return { countries: country ? [country] : [], missing: country ? [] : ['Recorded player country unavailable'] };
  }
  const tags = selection.kind === 'tags' ? selection.tags : [track.selectedTag!];
  return { countries: countries.filter((item) => tags.includes(item.tag)),
    missing: tags.filter((tag) => !countries.some((country) => country.tag === tag)).map((tag) => `${tag} unavailable in this recording`) };
}

/** Exact windows overlap so adjacent chunks are ready before the cursor arrives. */
export function exactWindow(track: BoundTrack, hour: number, next = false): HourWindow | null {
  if (hour < track.branch.firstHour || hour > track.branch.lastHour) return null;
  const start = track.branch.firstHour + Math.floor((hour - track.branch.firstHour) / PREFETCH_HOURS) * PREFETCH_HOURS
    + (next ? PREFETCH_HOURS : 0);
  return { firstHour: Math.max(track.branch.firstHour, start - TRAILING_HOURS + 1),
    lastHour: Math.min(track.branch.lastHour, start + 2 * PREFETCH_HOURS - 1) };
}

export function makeQueries(widget: WidgetSpec, track: BoundTrack, tracks: BoundTrack[], position: number, prefetch = false): DashboardQuery[] {
  const definition = widgetDefinition(widget.kind);
  if (!definition || definition.data !== 'countries' || widgetUnavailable(widget)) return [];
  const requested = resolveWindow(widget, track, tracks, position);
  if (!requested) return [];
  const exact = definition.scope === 'moment' || widget.time.mode === 'follow';
  if (prefetch && (!exact || widget.time.mode !== 'follow')) return [];
  const target = definition.scope === 'history' && widget.time.mode === 'follow' && requested.firstHour <= track.branch.lastHour
    ? Math.min(requested.lastHour, track.branch.lastHour) : requested.lastHour;
  const window = exact ? exactWindow(track, target, prefetch) : requested;
  if (!window) return [];
  const firstHour = Math.max(track.branch.firstHour, window.firstHour);
  const lastHour = Math.min(track.branch.lastHour, window.lastHour);
  if (firstHour > lastHour) return [];
  const { countries } = selectCountries(track, widget.countries);
  const requests: DashboardQuery[] = [];
  // Bound each response while retaining all selected countries across batches.
  for (let i = 0; i < countries.length; i += 24) {
    const countryIds = countries.slice(i, i + 24).map((item) => item.id);
    requests.push({ sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id,
      countryIds, firstHour, lastHour, kind: exact ? 'exact' : 'history',
      maxPoints: exact ? lastHour - firstHour + 1 : Math.min(600, Math.floor(1200 / countryIds.length)) });
  }
  return requests;
}

export function exactPoint(series: CountrySeries, hour: number): CountrySeriesPoint | null {
  // No nearest-observation or last-retained fallback: a gap stays a gap.
  return series.points.find((point) => point.firstHour === hour && point.lastHour === hour) ?? null;
}

/** Join normalized transport chunks, never join branches or invent gap samples. */
export function mergeSeriesChunks(chunks: CountrySeries[]): CountrySeries[] {
  const groups = new Map<string, CountrySeries[]>();
  for (const chunk of chunks) {
    const group = groups.get(chunk.countryId) ?? [];
    group.push(chunk);
    groups.set(chunk.countryId, group);
  }
  return [...groups.values()].map((group) => {
    const first = group[0];
    if (group.length === 1) return first;
    if (group.some((chunk) => chunk.branchId !== first.branchId || chunk.sessionId !== first.sessionId
      || chunk.revision !== first.revision || chunk.valueScale !== first.valueScale)) throw new Error('Country chunks have different source identities or value scales.');
    const points: CountrySeriesPoint[] = [];
    for (const point of group.flatMap((chunk) => chunk.points).sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour)) {
      const previous = points.at(-1);
      if (previous && previous.lastHour >= point.firstHour) {
        if (previous.firstHour === point.firstHour && previous.lastHour === point.lastHour && JSON.stringify(previous) === JSON.stringify(point)) continue;
        throw new Error('Country chunks contain conflicting or overlapping intervals. Refresh this recording.');
      }
      points.push(point);
    }
    const countries = new Map<string, CountryMetadata>();
    for (const chunk of group) for (const country of chunk.countries) {
      if ((countries.get(country.id)?.observedHour ?? -Infinity) <= country.observedHour) countries.set(country.id, country);
    }
    return { ...first, firstHour: Math.min(...group.map((chunk) => chunk.firstHour)), lastHour: Math.max(...group.map((chunk) => chunk.lastHour)),
      points, country: group.reduce<CountryMetadata | null>((country, chunk) => chunk.country ?? country, null), countries: [...countries.values()],
      coverage: points.reduce((coverage, point) => ({ recordedHours: coverage.recordedHours + point.recordedHours,
        gapHours: coverage.gapHours + point.gapHours, unavailableHours: coverage.unavailableHours + point.unavailableHours,
        absentHours: coverage.absentHours + point.absentHours }), { recordedHours: 0, gapHours: 0, unavailableHours: 0, absentHours: 0 }),
      downsampled: group.some((chunk) => chunk.downsampled), warnings: [...new Set(group.flatMap((chunk) => chunk.warnings))],
      sampleBoundary: [...new Set(group.map((chunk) => chunk.sampleBoundary).filter(Boolean))].join(' · '),
    };
  });
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

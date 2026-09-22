import { defaultTemplate, GRID_COLUMNS, WIDGET_CONTENT_SCALES } from './model.ts';
import type { Breakpoint, CountrySelection, DashboardTemplate, GridItem, GridLayouts, OffsetWindow, SourceRole, TrackSpec, WidgetSpec, WidgetTime } from './model';
import type { MetricKey } from '../analytics/model';
import { widgetDefinition } from './definitions.ts';
import { metrics } from '../analytics/model.ts';
import { MAX_ROLES, MAX_TRACKS } from './campaigns.ts';

const STORAGE_KEY = 'hoi4-replay.workspace-template.v1';
const LIBRARY_KEY = 'hoi4-replay.workspace-layouts.v1';
export const MAX_LAYOUTS = 32;
export interface LayoutLibrary {
  version: 1;
  activeId: string;
  layouts: { id: string; template: DashboardTemplate }[];
}
type LayoutStorage = Pick<Storage, 'getItem' | 'setItem'>;
const metricKeys = new Set<string>(metrics.map((metric) => metric.key));
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null && !Array.isArray(value);
const integer = (value: unknown, min = -4_294_967_296, max = 4_294_967_296): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= min && value <= max;
const text = (value: unknown, max = 160): value is string => typeof value === 'string' && value.length > 0 && value.length <= max;
const id = (value: unknown): value is string => text(value, 80) && /^[a-zA-Z0-9_-]+$/.test(value) && !['__proto__', 'constructor', 'prototype'].includes(value);

function offsetWindow(value: unknown): OffsetWindow | null {
  return object(value) && integer(value.firstOffset) && integer(value.lastOffset) && value.firstOffset <= value.lastOffset
    ? { firstOffset: value.firstOffset, lastOffset: value.lastOffset } : null;
}
function timeMode(value: unknown, tracks: Set<string>): WidgetTime | null {
  if (!object(value)) return null;
  if (value.mode === 'follow' || value.mode === 'campaign') return { mode: value.mode };
  if (value.mode === 'range') { const window = offsetWindow(value); return window ? { mode: 'range', ...window } : null; }
  if (value.mode !== 'pin' || !object(value.windows)) return null;
  const windows: Record<string, OffsetWindow> = {};
  for (const [trackId, input] of Object.entries(value.windows)) {
    const window = offsetWindow(input);
    if (!tracks.has(trackId) || !window) return null;
    windows[trackId] = window;
  }
  return { mode: 'pin', windows };
}
function selection(value: unknown): CountrySelection | null {
  if (!object(value)) return null;
  if (value.kind === 'selected' || value.kind === 'all') return { kind: value.kind };
  return value.kind === 'tags' && Array.isArray(value.tags) && value.tags.length <= 256 && value.tags.every((tag) => text(tag, 32))
    ? { kind: 'tags', tags: [...new Set(value.tags as string[])] } : null;
}

/** Parse an allowlist; unknown fields can never smuggle runtime data into storage. */
export function parseTemplate(input: unknown): DashboardTemplate | null {
  if (!object(input) || input.version !== 1 || !text(input.name) || !['calendar', 'anchors'].includes(String(input.alignment))) return null;
  if (!Array.isArray(input.roles) || !input.roles.length || input.roles.length > MAX_ROLES
    || !Array.isArray(input.tracks) || !input.tracks.length || input.tracks.length > MAX_TRACKS
    || !Array.isArray(input.widgets) || input.widgets.length > 48 || !object(input.layouts)) return null;
  const roles: SourceRole[] = [];
  for (const role of input.roles) {
    if (!object(role) || !id(role.id) || !text(role.label) || !(role.selectedTag === null || text(role.selectedTag, 32)) || roles.some((item) => item.id === role.id)) return null;
    roles.push({ id: role.id, label: role.label, selectedTag: role.selectedTag });
  }
  if (!roles.some((role) => role.id === 'primary')) return null;
  const tracks: TrackSpec[] = [];
  for (const track of input.tracks) {
    if (!object(track) || !id(track.id) || !text(track.label) || !roles.some((role) => role.id === track.roleId)
      || !integer(track.branchIndex, 0, 65535) || !(track.anchorOffset === null || integer(track.anchorOffset, 0)) || tracks.some((item) => item.id === track.id)) return null;
    tracks.push({ id: track.id, roleId: track.roleId as string, label: track.label, branchIndex: track.branchIndex, anchorOffset: track.anchorOffset });
  }
  const trackIds = new Set(tracks.map((track) => track.id));
  const widgets: WidgetSpec[] = [];
  for (const widget of input.widgets) {
    if (object(widget) && widget.campaign !== undefined && widget.campaign !== 'default' && widget.campaign !== 'tracks') return null;
    if (!object(widget) || !id(widget.id) || !text(widget.title) || !id(widget.kind)
      || !(widget.configVersion === undefined || integer(widget.configVersion, 1, 65535))
      || !(widget.contentScale === undefined || WIDGET_CONTENT_SCALES.some(scale => scale === widget.contentScale))
      || !Array.isArray(widget.trackIds) || !widget.trackIds.length || !widget.trackIds.every((value) => typeof value === 'string' && trackIds.has(value))
      || !metricKeys.has(String(widget.metric)) || widgets.some((item) => item.id === widget.id)) return null;
    const countries = selection(widget.countries);
    const time = timeMode(widget.time, trackIds);
    if (!countries || !time) return null;
    const campaign = widget.campaign === 'default' || (widget.campaign === undefined && widget.trackIds.length === 1 && widget.trackIds[0] === 'main' && time.mode !== 'pin') ? 'default' : 'tracks';
    widgets.push({ campaign, id: widget.id, kind: widget.kind, configVersion: widget.configVersion ?? 1, title: widget.title,
      trackIds: [...new Set(widget.trackIds as string[])], metric: widget.metric as MetricKey, countries, time, contentScale: (widget.contentScale as number | undefined) ?? 100 });
  }
  const layouts: GridLayouts = {};
  for (const breakpoint of Object.keys(GRID_COLUMNS) as Breakpoint[]) {
    const cols = GRID_COLUMNS[breakpoint];
    const inputLayout = input.layouts[breakpoint];
    if (inputLayout == null) continue;
    if (!Array.isArray(inputLayout) || inputLayout.length !== widgets.length) return null;
    const layout: GridItem[] = [];
    for (const item of inputLayout) {
      if (!object(item) || !widgets.some((widget) => widget.id === item.i) || layout.some((existing) => existing.i === item.i)
        || !integer(item.x, 0, cols - 1) || !integer(item.y, 0, 10000) || !integer(item.w, 1, cols) || !integer(item.h, 1, 100)
        || item.x + item.w > cols) return null;
      const definition = widgetDefinition(widgets.find((widget) => widget.id === item.i)!.kind);
      layout.push({ i: item.i as string, x: item.x, y: item.y, w: item.w, h: item.h,
        minW: Math.min(cols, definition?.minSize.w ?? 1), minH: definition?.minSize.h ?? 4 });
    }
    layouts[breakpoint] = layout;
  }
  if (!layouts.lg) return null;
  return { version: 1, name: input.name, alignment: input.alignment as DashboardTemplate['alignment'], roles, tracks, widgets, layouts };
}

export function parseLayoutLibrary(input: unknown): LayoutLibrary | null {
  if (!object(input) || input.version !== 1 || !id(input.activeId) || !Array.isArray(input.layouts)
    || !input.layouts.length || input.layouts.length > MAX_LAYOUTS) return null;
  const layouts: LayoutLibrary['layouts'] = [];
  for (const entry of input.layouts) {
    if (!object(entry) || !id(entry.id) || layouts.some((item) => item.id === entry.id)) return null;
    const template = parseTemplate(entry.template);
    if (!template || !template.name.trim()) return null;
    template.name = template.name.trim();
    if (layouts.some((item) => item.template.name.toLowerCase() === template.name.toLowerCase())) return null;
    layouts.push({ id: entry.id, template });
  }
  return layouts.some((entry) => entry.id === input.activeId) ? { version: 1, activeId: input.activeId, layouts } : null;
}

function migrateDefaultName(library: LayoutLibrary): LayoutLibrary {
  if (library.layouts.some((entry) => entry.template.name.toLowerCase() === 'default layout')) return library;
  return { ...library, layouts: library.layouts.map((entry) => entry.id === 'default' && entry.template.name === 'Campaign overview'
    ? { ...entry, template: { ...entry.template, name: 'Default layout' } } : entry) };
}

export function loadLayouts(storage?: LayoutStorage): { library: LayoutLibrary; warning: string | null } {
  const fallback: LayoutLibrary = { version: 1, activeId: 'default', layouts: [{ id: 'default', template: { ...defaultTemplate(), name: 'Default layout' } }] };
  try {
    const target = storage ?? localStorage;
    const raw = target.getItem(LIBRARY_KEY);
    if (raw !== null) {
      if (raw.length > 4_000_000) throw new Error('Saved layouts are too large.');
      const library = parseLayoutLibrary(JSON.parse(raw));
      if (!library) throw new Error('Invalid layouts');
      return { library: migrateDefaultName(library), warning: null };
    }
    // Restore the previous single-layout format without overwriting it.
    const legacy = target.getItem(STORAGE_KEY);
    if (legacy !== null) {
      if (legacy.length > 1_000_000) throw new Error('Saved layout is too large.');
      const library = parseLayoutLibrary({ ...fallback, layouts: [{ id: 'default', template: JSON.parse(legacy) }] });
      if (!library) throw new Error('Invalid layout');
      return { library: migrateDefaultName(library), warning: null };
    }
    return { library: fallback, warning: null };
  } catch {
    return { library: fallback, warning: 'Saved layouts could not be restored. A default layout is open; your recording is unaffected.' };
  }
}

export function saveLayouts(library: LayoutLibrary, storage?: LayoutStorage): string | null {
  try {
    const safe = parseLayoutLibrary(library);
    if (!safe) throw new Error('Invalid layouts');
    const raw = JSON.stringify(safe);
    if (raw.length > 4_000_000) throw new Error('Saved layouts are too large.');
    (storage ?? localStorage).setItem(LIBRARY_KEY, raw);
    return null;
  } catch {
    return 'This layout could not be saved on this device. Your current workspace is still available.';
  }
}

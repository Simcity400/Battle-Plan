import { defaultTemplate, GRID_COLUMNS, migrateWidget, stableJson, WIDGET_CONTENT_SCALES } from './model.ts';
import { parseStoredOptions } from './options.ts';
import type { Breakpoint, CountryGroup, CountrySelection, DashboardTemplate, GridItem, GridLayouts, OffsetWindow, SourceRole, TrackSpec, WidgetSpec, WidgetTime } from './model';
import { widgetDefinition } from './definitions.ts';
import type { PageId } from './pages.ts';

const STORAGE_KEY = 'battleplan.workspace-template.v1';
const LIBRARY_KEY = 'battleplan.workspace-layouts.v1';
export const MAX_LAYOUTS = 32;
export interface LayoutLibrary {
  version: 1;
  activeId: string;
  layouts: { id: string; template: DashboardTemplate }[];
}
type LayoutStorage = Pick<Storage, 'getItem' | 'setItem'>;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null && !Array.isArray(value);
const integer = (value: unknown, min = -4_294_967_296, max = 4_294_967_296): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= min && value <= max;
const text = (value: unknown, max = 160): value is string => typeof value === 'string' && value.length > 0 && value.length <= max;
const widgetTitle = (value: unknown): value is string => typeof value === 'string';
const id = (value: unknown): value is string => text(value, 80) && /^[a-zA-Z0-9_-]+$/.test(value) && !['__proto__', 'constructor', 'prototype'].includes(value);
export const storableTag = (value: unknown): value is string => typeof value === 'string' && value.length > 0
  && !value.includes('\0');

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
function countryGroup(value: unknown): CountryGroup | null {
  if (!object(value)) return null;
  if (value.type === 'humans') return { type: 'humans' };
  if (value.type === 'faction' && typeof value.id === 'string' && /^\d{1,20}$/.test(value.id)) return { type: 'faction', id: value.id };
  if (value.type === 'war' && storableTag(value.tag)) return { type: 'war', tag: value.tag };
  if (value.type === 'top' && integer(value.count, 1, Number.MAX_SAFE_INTEGER)) return { type: 'top', count: value.count };
  return null;
}
function selection(value: unknown, dropped?: (count: number) => void): CountrySelection | null {
  if (!object(value)) return null;
  if (value.kind === 'selected' || value.kind === 'all') return { kind: value.kind };
  if (value.kind === 'factions') return integer(value.count, 1, Number.MAX_SAFE_INTEGER) && (value.membership === 'hour' || value.membership === 'end' || value.membership === 'ever')
    ? { kind: 'factions', count: value.count, membership: value.membership } : null;
  if (value.kind === 'group') {
    const group = countryGroup(value.group);
    return group && (value.membership === 'hour' || value.membership === 'end' || value.membership === 'ever') && typeof value.aggregate === 'boolean'
      ? { kind: 'group', group, membership: value.membership, aggregate: value.aggregate } : null;
  }
  if (value.kind !== 'tags' || !Array.isArray(value.tags)) return null;
  const tags = value.tags.filter(storableTag);
  dropped?.(value.tags.length - tags.length);
  return { kind: 'tags', tags: [...new Set(tags)] };
}

function safeMigrate(widget: WidgetSpec, trackIds: Set<string>): WidgetSpec {
  let migrated: WidgetSpec;
  try { migrated = migrateWidget(widget); } catch { return widget; }
  if (migrated === widget) return widget;
  const valid = widgetTitle(migrated.title) && migrated.trackIds.length > 0 && migrated.trackIds.every(value => trackIds.has(value))
    && selection(migrated.countries) && timeMode(migrated.time, trackIds)
    && (migrated.options === undefined || parseStoredOptions(migrated.options))
    && (migrated.contentScale === undefined || WIDGET_CONTENT_SCALES.some(scale => scale === migrated.contentScale));
  return valid ? migrated : widget;
}

export function parseTemplate(input: unknown): DashboardTemplate | null {
  return parseTemplateWithNotes(input)?.template ?? null;
}

export function parseTemplateWithNotes(input: unknown): { template: DashboardTemplate; droppedTags: number } | null {
  let droppedTags = 0;
  if (!object(input) || input.version !== 1 || !text(input.name) || !['calendar', 'anchors'].includes(String(input.alignment))) return null;
  if (!Array.isArray(input.roles) || !input.roles.length
    || !Array.isArray(input.tracks) || !input.tracks.length
    || !Array.isArray(input.widgets) || !object(input.layouts)) return null;
  const roles: SourceRole[] = [], roleIds = new Set<string>();
  for (const role of input.roles) {
    if (!object(role) || !id(role.id) || !text(role.label) || !(role.selectedTag === null || typeof role.selectedTag === 'string') || roleIds.has(role.id)) return null;
    if (role.selectedTag !== null && !storableTag(role.selectedTag)) droppedTags++;
    roles.push({ id: role.id, label: role.label, selectedTag: storableTag(role.selectedTag) ? role.selectedTag : null });
    roleIds.add(role.id);
  }
  if (!roleIds.has('primary')) return null;
  const tracks: TrackSpec[] = [], trackById = new Map<string, TrackSpec>();
  for (const track of input.tracks) {
    if (!object(track) || !id(track.id) || !text(track.label) || !roleIds.has(track.roleId as string)
      || !integer(track.branchIndex, 0, 65535) || !(track.anchorOffset === null || integer(track.anchorOffset, 0)) || trackById.has(track.id as string)) return null;
    const stored = { id: track.id, roleId: track.roleId as string, label: track.label, branchIndex: track.branchIndex, anchorOffset: track.anchorOffset };
    tracks.push(stored); trackById.set(stored.id, stored);
  }
  const trackIds = new Set(trackById.keys());
  const widgets: WidgetSpec[] = [], widgetById = new Map<string, WidgetSpec>();
  for (const widget of input.widgets) {
    if (object(widget) && widget.campaign !== undefined && widget.campaign !== 'default' && widget.campaign !== 'tracks') return null;
    if (!object(widget) || !id(widget.id) || !widgetTitle(widget.title) || !id(widget.kind)
      || !(widget.configVersion === undefined || integer(widget.configVersion, 1, 65535))
      || !(widget.contentScale === undefined || WIDGET_CONTENT_SCALES.some(scale => scale === widget.contentScale))
      || !Array.isArray(widget.trackIds) || !widget.trackIds.length || !widget.trackIds.every((value) => typeof value === 'string' && trackIds.has(value))
      || widgetById.has(widget.id as string)) return null;
    const countries = selection(widget.countries, count => { droppedTags += count; });
    const time = timeMode(widget.time, trackIds);
    const options = widget.options === undefined ? undefined : parseStoredOptions(widget.options);
    if (!countries || !time || options === null) return null;
    const campaign = widget.campaign === 'default' || (widget.campaign === undefined && widget.trackIds.length === 1 && widget.trackIds[0] === 'main' && time.mode !== 'pin') ? 'default' : 'tracks';
    const stored: WidgetSpec = { campaign, id: widget.id, kind: widget.kind, configVersion: widget.configVersion ?? 1, title: widget.title,
      trackIds: [...new Set(widget.trackIds as string[])], countries, time, contentScale: (widget.contentScale as number | undefined) ?? 100,
      ...(options && Object.keys(options).length ? { options } : {}) };
    const migrated = safeMigrate(stored, trackIds);
    widgets.push(migrated); widgetById.set(migrated.id, migrated);
  }
  const layouts: GridLayouts = {};
  for (const breakpoint of Object.keys(GRID_COLUMNS) as Breakpoint[]) {
    const cols = GRID_COLUMNS[breakpoint];
    const inputLayout = input.layouts[breakpoint];
    if (inputLayout == null) continue;
    if (!Array.isArray(inputLayout) || inputLayout.length !== widgets.length) return null;
    const layout: GridItem[] = [], layoutIds = new Set<string>();
    for (const item of inputLayout) {
      if (!object(item) || !widgetById.has(item.i as string) || layoutIds.has(item.i as string)
        || !integer(item.x, 0, cols - 1) || !integer(item.y, 0, 10000) || !integer(item.w, 1, cols) || !integer(item.h, 1, 100)
        || item.x + item.w > cols) return null;
      const definition = widgetDefinition(widgetById.get(item.i as string)!.kind);
      layout.push({ i: item.i as string, x: item.x, y: item.y, w: item.w, h: item.h,
        minW: Math.min(cols, definition?.minSize.w ?? 1), minH: definition?.minSize.h ?? 4 });
      layoutIds.add(item.i as string);
    }
    layouts[breakpoint] = layout;
  }
  if (!layouts.lg) return null;
  return { template: { version: 1, name: input.name, alignment: input.alignment as DashboardTemplate['alignment'], roles, tracks, widgets, layouts }, droppedTags };
}

export function parseLayoutLibrary(input: unknown): LayoutLibrary | null {
  return parseLayoutLibraryWithNotes(input)?.library ?? null;
}

export function parseLayoutLibraryWithNotes(input: unknown): { library: LayoutLibrary; droppedTags: number } | null {
  let droppedTags = 0;
  if (!object(input) || input.version !== 1 || !id(input.activeId) || !Array.isArray(input.layouts)
    || !input.layouts.length || input.layouts.length > MAX_LAYOUTS) return null;
  const layouts: LayoutLibrary['layouts'] = [], layoutIds = new Set<string>(), layoutNames = new Set<string>();
  for (const entry of input.layouts) {
    if (!object(entry) || !id(entry.id) || layoutIds.has(entry.id)) return null;
    const parsed = parseTemplateWithNotes(entry.template);
    if (!parsed || !parsed.template.name.trim()) return null;
    const { template } = parsed;
    droppedTags += parsed.droppedTags;
    template.name = template.name.trim();
    const name = template.name.toLowerCase();
    if (layoutNames.has(name)) return null;
    layouts.push({ id: entry.id, template });
    layoutIds.add(entry.id); layoutNames.add(name);
  }
  return layouts.some((entry) => entry.id === input.activeId) ? { library: { version: 1, activeId: input.activeId, layouts }, droppedTags } : null;
}

function migrateDefaultName(library: LayoutLibrary): LayoutLibrary {
  if (library.layouts.some((entry) => entry.template.name.toLowerCase() === 'default layout')) return library;
  return { ...library, layouts: library.layouts.map((entry) => entry.id === 'default' && entry.template.name === 'Campaign overview'
    ? { ...entry, template: { ...entry.template, name: 'Default layout' } } : entry) };
}

export function droppedTagWarning(count: number): string | null {
  return count ? `${count} country tag ${count === 1 ? 'entry was' : 'entries were'} left out of the saved layout because ${count === 1 ? 'it' : 'they'} couldn’t be stored.` : null;
}

export type LayoutSaveAction = 'update' | 'new';
export interface LayoutSaveCheck { submit: LayoutSaveAction | null; update: boolean; asNew: boolean; renames: boolean; problem: string | null }

export function layoutSaveCheck(library: LayoutLibrary, typed: string): LayoutSaveCheck {
  const name = typed.trim();
  const current = library.layouts.find(entry => entry.id === library.activeId);
  const blocked = (problem: string | null): LayoutSaveCheck => ({ submit: null, update: false, asNew: false, renames: false, problem });
  if (!name) return blocked('Enter a layout name.');
  if (name.length > 160) return blocked('Use 160 characters or fewer.');
  const taken = library.layouts.find(entry => entry.id !== library.activeId && entry.template.name.toLowerCase() === name.toLowerCase());
  if (taken) return blocked(`“${taken.template.name}” is already a layout. Choose another name.`);
  if (current && current.template.name.toLowerCase() === name.toLowerCase()) {
    return { submit: 'update', update: true, asNew: false, renames: current.template.name !== name, problem: null };
  }
  const room = library.layouts.length < MAX_LAYOUTS;
  return { submit: room ? 'new' : null, update: !!current, asNew: room, renames: !!current,
    problem: room ? null : `You can keep up to ${MAX_LAYOUTS} layouts. Update this one instead.` };
}

export const layoutSaveAllowed = (check: LayoutSaveCheck, action: LayoutSaveAction) => action === 'new' ? check.asNew : check.update;

const libraryKey = (page: PageId) => page === 'overview' ? LIBRARY_KEY : `${LIBRARY_KEY}.${page}`;

const defaultLibrary = (page: PageId): LayoutLibrary =>
  ({ version: 1, activeId: 'default', layouts: [{ id: 'default', template: { ...defaultTemplate(page), name: 'Default layout' } }] });

export function layoutsAtDefault(page: PageId, storage?: LayoutStorage): boolean {
  const { library, warning } = loadLayouts(storage, page);
  return warning === null && stableJson(library) === stableJson(parseLayoutLibrary(defaultLibrary(page)));
}

interface PageDraft { library: LayoutLibrary; template: DashboardTemplate }
type Merge = (template: DashboardTemplate) => DashboardTemplate;

function draftUnsaved({ library, template }: PageDraft, merge: Merge): boolean {
  const active = library.layouts.find(entry => entry.id === library.activeId)?.template;
  return !active || stableJson(merge(template)) !== stableJson(merge(active));
}

export function unsavedLayoutPages<K>(pages: Iterable<readonly [K, PageDraft]>, merge: Merge = template => template): K[] {
  return [...pages].filter(([, page]) => draftUnsaved(page, merge)).map(([id]) => id);
}

export function doneEditing(page: PageDraft, merge: Merge = template => template): 'save' | 'leave' {
  return draftUnsaved(page, merge) ? 'save' : 'leave';
}

export function loadLayouts(storage?: LayoutStorage, page: PageId = 'overview'): { library: LayoutLibrary; warning: string | null; droppedTags: number } {
  const fallback = defaultLibrary(page);
  try {
    const target = storage ?? localStorage;
    const raw = target.getItem(libraryKey(page));
    if (raw !== null) {
      if (raw.length > 4_000_000) throw new Error('Saved layouts are too large.');
      const parsed = parseLayoutLibraryWithNotes(JSON.parse(raw));
      if (!parsed) throw new Error('Invalid layouts');
      return { library: migrateDefaultName(parsed.library), warning: droppedTagWarning(parsed.droppedTags), droppedTags: parsed.droppedTags };
    }
    const legacy = page === 'overview' ? target.getItem(STORAGE_KEY) : null;
    if (legacy !== null) {
      if (legacy.length > 1_000_000) throw new Error('Saved layout is too large.');
      const parsed = parseLayoutLibraryWithNotes({ ...fallback, layouts: [{ id: 'default', template: JSON.parse(legacy) }] });
      if (!parsed) throw new Error('Invalid layout');
      return { library: migrateDefaultName(parsed.library), warning: droppedTagWarning(parsed.droppedTags), droppedTags: parsed.droppedTags };
    }
    return { library: fallback, warning: null, droppedTags: 0 };
  } catch {
    return { library: fallback, warning: 'Couldn’t restore saved layouts. A default layout is open; your recording is unaffected.', droppedTags: 0 };
  }
}

export function clearLayouts(page: PageId, storage?: Pick<Storage, 'removeItem'>) {
  try {
    const target = storage ?? localStorage;
    target.removeItem(libraryKey(page));
    if (page === 'overview') target.removeItem(STORAGE_KEY);
  } catch {  }
}

export function saveLayouts(library: LayoutLibrary, storage?: LayoutStorage, page: PageId = 'overview'): string | null {
  try {
    const parsed = parseLayoutLibraryWithNotes(library);
    if (!parsed) throw new Error('Invalid layouts');
    if (parsed.droppedTags) return `${parsed.droppedTags} country tag entries couldn’t be saved. Your current workspace is still available.`;
    const raw = JSON.stringify(parsed.library);
    if (raw.length > 4_000_000) throw new Error('Saved layouts are too large.');
    (storage ?? localStorage).setItem(libraryKey(page), raw);
    return null;
  } catch {
    return 'Couldn’t save this layout on this device. Your changes are still here.';
  }
}

import { widgetDefinition, type WidgetMeta, type WidgetRenderer, type WidgetViewSwitch } from '../dashboard/definitions.ts';
import type { OptionField, WidgetOptions } from '../dashboard/options';
import {
  DEFAULT_VIEW,
  manifestViewSwitch,
  normalizeView,
  resolveActiveView,
  scopedOptionFields,
  viewKey,
  type FamilyViewEntry,
  type FamilyViewModule,
  type LegacyWidgetEntry,
} from '../families/manifest.ts';
import { FAMILY_VIEWS } from './familyManifest.ts';
import { LEGACY_WIDGETS } from './legacyManifest.ts';

export { DEFAULT_VIEW, FAMILY_VIEWS, LEGACY_WIDGETS };

export interface FamilyViewUiLoader {
  readonly load: () => Promise<{ default: WidgetRenderer }>;
  readonly slots?: readonly ('render' | 'countryChip')[];
}

export const FAMILY_VIEW_UI: Readonly<Record<string, FamilyViewUiLoader>> = {
  'apm-table': { load: () => import('./apm-table') },
  'bombing-damage-breakdown': { load: () => import('./bombing-damage-breakdown'), slots: ['render', 'countryChip'] },
  'casualties-breakdown': { load: () => import('./casualties-breakdown') },
  'construction-spending': { load: () => import('./construction-spending') },
  'division-template': { load: () => import('./division-template') },
  'dockyard-spending': { load: () => import('./dockyard-spending') },
  'equipment-losses': { load: () => import('./equipment-losses'), slots: ['render', 'countryChip'] },
  'game-score': { load: () => import('./game-score') },
  'military-factory-spending': { load: () => import('./military-factory-spending') },
  'occupied-states': { load: () => import('./occupied-states') },
  'political-power': { load: () => import('./political-power'), slots: ['render', 'countryChip'] },
  'political-power-spending-breakdown': { load: () => import('./political-power-spending-breakdown') },
  'production-spending': { load: () => import('./production-spending') },
  'scoreboard': { load: () => import('./scoreboard') },
  'territory': { load: () => import('./territory') },
  'territory-breakdown': { load: () => import('./territory-breakdown') },
};

const familyByKind = new Map<string, FamilyViewEntry[]>();
const familyByPair = new Map<string, FamilyViewEntry>();
const legacyByKind = new Map<string, LegacyWidgetEntry[]>();
const legacyByPair = new Map<string, LegacyWidgetEntry>();

for (const entry of FAMILY_VIEWS) {
  let list = familyByKind.get(entry.kind);
  if (!list) familyByKind.set(entry.kind, list = []);
  list.push(entry);
  familyByPair.set(viewKey(entry.kind, entry.view), entry);
}

for (const entry of LEGACY_WIDGETS) {
  let list = legacyByKind.get(entry.kind);
  if (!list) legacyByKind.set(entry.kind, list = []);
  list.push(entry);
  legacyByPair.set(viewKey(entry.kind, entry.view), entry);
}

const EMPTY_FAMILY: readonly FamilyViewEntry[] = [];
const EMPTY_LEGACY: readonly LegacyWidgetEntry[] = [];

export function familyViewsForKind(kind: string): readonly FamilyViewEntry[] {
  return familyByKind.get(kind) ?? EMPTY_FAMILY;
}

export function legacyEntriesForKind(kind: string): readonly LegacyWidgetEntry[] {
  return legacyByKind.get(kind) ?? EMPTY_LEGACY;
}

export function manifestFamilyView(kind: string, view?: string): FamilyViewEntry | undefined {
  return familyByPair.get(viewKey(kind, view));
}

export function manifestLegacyEntry(kind: string, view?: string): LegacyWidgetEntry | undefined {
  return legacyByPair.get(viewKey(kind, view)) ?? legacyByPair.get(viewKey(kind, DEFAULT_VIEW));
}

export function resolveWidgetActiveView(
  kind: string,
  storedOptions?: WidgetOptions,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options'>,
): string {
  const meta = metaOverride ?? widgetDefinition(kind);
  return resolveActiveView(storedOptions, meta, familyViewsForKind(kind), legacyEntriesForKind(kind));
}

export function activeFamilyView(
  kind: string,
  storedOptions?: WidgetOptions,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options'>,
): FamilyViewEntry | undefined {
  const view = resolveWidgetActiveView(kind, storedOptions, metaOverride);
  return familyByPair.get(viewKey(kind, view));
}

export function isFamilyWidgetView(
  kind: string,
  storedOptions?: WidgetOptions,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options'>,
): boolean {
  return activeFamilyView(kind, storedOptions, metaOverride) !== undefined;
}

export function isPlaybackDatedView(
  kind: string,
  storedOptions?: WidgetOptions,
  definition?: Pick<WidgetMeta, 'playsBackDated' | 'viewSwitch' | 'options'>,
): boolean {
  const view = resolveWidgetActiveView(kind, storedOptions, definition);
  if (familyByPair.has(viewKey(kind, view))) return true;
  const legacy = legacyByPair.get(viewKey(kind, view)) ?? legacyByPair.get(viewKey(kind, DEFAULT_VIEW));
  if (legacy) return legacy.playsBackDated === true || definition?.playsBackDated === true;
  return definition?.playsBackDated === true;
}

export function resolveWidgetScopedOptions(
  kind: string,
  storedOptions?: WidgetOptions,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options'>,
): readonly OptionField[] {
  const meta = metaOverride ?? widgetDefinition(kind);
  const familyEntries = familyViewsForKind(kind);
  const legacyEntries = legacyEntriesForKind(kind);
  const activeView = resolveActiveView(storedOptions, meta, familyEntries, legacyEntries);
  return scopedOptionFields(meta, activeView, familyEntries, legacyEntries);
}

export function resolveWidgetViewSwitch(
  kind: string,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options' | 'title'>,
): WidgetViewSwitch | undefined {
  const meta = metaOverride ?? widgetDefinition(kind);
  return manifestViewSwitch(meta, familyViewsForKind(kind), legacyEntriesForKind(kind));
}

export function manifestKinds(): readonly string[] {
  const seen = new Set<string>();
  const kinds: string[] = [];
  for (const entry of FAMILY_VIEWS) {
    if (!seen.has(entry.kind)) {
      seen.add(entry.kind);
      kinds.push(entry.kind);
    }
  }
  for (const entry of LEGACY_WIDGETS) {
    if (!seen.has(entry.kind)) {
      seen.add(entry.kind);
      kinds.push(entry.kind);
    }
  }
  return kinds;
}

export function manifestSlotsForKind(kind: string): readonly ('render' | 'countryChip')[] {
  const ui = Object.hasOwn(FAMILY_VIEW_UI, kind) ? FAMILY_VIEW_UI[kind] : undefined;
  if (ui?.slots?.length) return ui.slots;
  const legacy = legacyEntriesForKind(kind);
  for (const entry of legacy) {
    if (entry.slots?.length) return entry.slots;
  }
  return ['render'];
}

const loadedSpecs = new Map<string, Promise<FamilyViewModule | undefined>>();
const readySpecs = new Map<string, FamilyViewModule>();

export function loadedFamilyViewSpec(kind: string, view?: string): FamilyViewModule | undefined {
  return readySpecs.get(viewKey(kind, view));
}

export function loadFamilyViewSpec(kind: string, view?: string): Promise<FamilyViewModule | undefined> {
  const key = viewKey(kind, normalizeView(view));
  const entry = familyByPair.get(key);
  if (!entry) return Promise.resolve(undefined);
  let held = loadedSpecs.get(key);
  if (!held) {
    held = entry.loadSpec().then(mod => {
      readySpecs.set(key, mod);
      return mod;
    });
    loadedSpecs.set(key, held);
    held.catch(() => { loadedSpecs.delete(key); });
  }
  return held;
}

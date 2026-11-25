import { widgetDefinition, type FamilyViewRenderer, type WidgetMeta, type WidgetRenderer, type WidgetViewSwitch } from '../dashboard/definitions.ts';
import { resolveOptions, type OptionField, type OptionValue, type WidgetOptions } from '../dashboard/options.ts';
import {
  DEFAULT_VIEW,
  manifestViewSwitch,
  normalizeView,
  resolveActiveView,
  scopedOptionFields,
  viewKey,
  viewOwnedFields,
  type FamilyViewEntry,
  type FamilyViewModule,
  type LegacyWidgetEntry,
} from '../families/manifest.ts';
import { FAMILY_VIEWS } from './familyManifest.ts';
import { LEGACY_WIDGETS } from './legacyManifest.ts';

export { DEFAULT_VIEW, FAMILY_VIEWS, LEGACY_WIDGETS };

export type OptionChoicesHook = NonNullable<WidgetRenderer['useOptionChoices']>;

export const OPTION_CHOICES: Readonly<Record<string, () => Promise<OptionChoicesHook>>> = {
  'air-kills-losses': () => import('../dashboard/useMilitaryAirLedger').then(module => module.useAirLossChoices),
  'equipment-produced': () => import('./equipment-produced/useProduced').then(module => module.useProducedTypeChoices),
  'equipment-stockpile': () => import('./equipment-stockpile/useStockpileChoices').then(module => module.useStockpileChoices),
};

export interface FamilyViewUiLoader {
  readonly load: () => Promise<{ default: FamilyViewRenderer }>;
  readonly slots?: readonly ('render' | 'countryChip' | 'subjectMenu')[];
  readonly views?: readonly string[];
}

export const FAMILY_VIEW_UI: Readonly<Record<string, FamilyViewUiLoader>> = {
  'active-research': { load: () => import('./active-research') },
  'agency-comparison': { load: () => import('./agency-comparison') },
  'air-kills-losses': { load: () => import('./air-kills-losses/family.tsx') },
  'air-statistics': { load: () => import('./air-statistics') },
  'air-superiority-region': { load: () => import('./air-superiority-region') },
  'apm-graph': { load: () => import('./apm-graph') },
  'apm-table': { load: () => import('./apm-table') },
  'army-losses-by-month': { load: () => import('./army-losses-by-month') },
  'army-size': { load: () => import('./army-size/family.tsx') },
  'command-power': { load: () => import('./command-power'), slots: ['render', 'countryChip'] },
  'decisions': { load: () => import('./decisions') },
  'division-template': { load: () => import('./division-template') },
  'doctrine-mastery': { load: () => import('./doctrine-mastery') },
  'general-career': { load: () => import('./general-career/family.ts'), views: ['dossier'] },
  'equipment-designs': { load: () => import('./equipment-designs') },
  'equipment-produced': { load: () => import('./equipment-produced/family.tsx') },
  'event-choices': { load: () => import('./event-choices') },
  'event-feed': { load: () => import('./event-feed') },
  'factions': { load: () => import('./factions') },
  'faction-strength': { load: () => import('./faction-strength/family.tsx'), views: ['lines'] },
  'fuel': { load: () => import('./fuel') },
  'factories': { load: () => import('./factories') },
  'generals': { load: () => import('./generals/family.tsx') },
  'idea-timeline': { load: () => import('./idea-timeline') },
  'industrial-organisations': { load: () => import('./industrial-organisations') },
  'international-market': { load: () => import('./international-market') },
  'minefields': { load: () => import('./minefields') },
  'navy-size': { load: () => import('./navy-size') },
  'occupation-reserve-manpower': { load: () => import('./occupation-reserve-manpower/family.tsx') },
  'political-power': { load: () => import('./political-power'), slots: ['render', 'countryChip'] },
  'production-lines': { load: () => import('./production-lines') },
  'raid-log': { load: () => import('./raid-log') },
  'research': { load: () => import('./research') },
  'scoreboard': { load: () => import('./scoreboard') },
  'special-projects': { load: () => import('./special-projects'), views: ['live'] },
  'stability': { load: () => import('./stability') },
  'territory': { load: () => import('./territory/family.tsx'), views: ['lines'] },
  'war-contribution': { load: () => import('./war-contribution'), slots: ['render', 'countryChip', 'subjectMenu'] },
  'convoy-health': { load: () => import('./convoy-health'), slots: ['render', 'countryChip'] },
  'air-superiority-timeline': { load: () => import('./air-superiority-timeline'), slots: ['render', 'countryChip'] },
  'weather-by-region': { load: () => import('./weather-by-region') },
  'naval-supremacy': { load: () => import('./naval-supremacy'), slots: ['render', 'countryChip'] },
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

const viewFields = new Map<string, readonly OptionField[]>();

export function familyViewOptionFields(kind: string, view?: string): readonly OptionField[] {
  const key = viewKey(kind, view);
  const held = viewFields.get(key);
  if (held) return held;
  const definition = widgetDefinition(kind);
  const fields = new Map<string, OptionField>();
  for (const field of definition?.options ?? []) fields.set(field.key, field);
  for (const field of familyByPair.get(key)?.options ?? []) fields.set(field.key, field);
  const owned = viewOwnedFields([...fields.values()], view);
  if (definition) viewFields.set(key, owned);
  return owned;
}

function viewOptionFields(kind: string, view: string): readonly OptionField[] {
  return familyByPair.has(viewKey(kind, view)) ? familyViewOptionFields(kind, view) : widgetDefinition(kind)?.options ?? [];
}

export function widgetOptionFields(kind: string, storedOptions?: WidgetOptions): readonly OptionField[] {
  return viewOptionFields(kind, resolveWidgetActiveView(kind, storedOptions));
}

export type ResolvedWidgetOptions = Readonly<Record<string, OptionValue>>;

const NO_STORED: WidgetOptions = Object.freeze({});
const resolvedByStored = new WeakMap<WidgetOptions, Map<string, ResolvedWidgetOptions>>();

export function widgetViewOptions(kind: string, storedOptions?: WidgetOptions, view = resolveWidgetActiveView(kind, storedOptions)): ResolvedWidgetOptions {
  const stored = storedOptions ?? NO_STORED;
  let byView = resolvedByStored.get(stored);
  if (!byView) resolvedByStored.set(stored, byView = new Map());
  const key = viewKey(kind, view);
  let resolved = byView.get(key);
  if (!resolved) byView.set(key, resolved = Object.freeze(resolveOptions(viewOptionFields(kind, view), stored)));
  return resolved;
}

export interface PresetWidget { readonly title?: string; readonly options?: WidgetOptions }

const presetWidgets = new WeakMap<PresetWidget, Map<string, PresetWidget>>();

export function presetWidget(kind: string, initial: PresetWidget): PresetWidget {
  let byKind = presetWidgets.get(initial);
  if (!byKind) presetWidgets.set(initial, byKind = new Map());
  let held = byKind.get(kind);
  if (held) return held;
  const chosen = initial.options;
  if (chosen) {
    const fields = viewOptionFields(kind, resolveWidgetActiveView(kind, chosen));
    const stored: Record<string, OptionValue> = {};
    for (const [key, value] of Object.entries(chosen)) stored[fields.find(field => field.key === key)?.storeAs ?? key] = value;
    held = { ...initial, options: stored };
  } else held = initial;
  byKind.set(kind, held);
  return held;
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

export function manifestSlotsForKind(kind: string): readonly ('render' | 'countryChip' | 'subjectMenu')[] {
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

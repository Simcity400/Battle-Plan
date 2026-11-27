import { widgetDefinition, type FamilyViewRenderer, type WidgetMeta, type WidgetRenderer, type WidgetViewSwitch } from '../dashboard/definitions.ts';
import { resolveOptions, type OptionField, type OptionValue, type WidgetOptions } from '../dashboard/options.ts';
import {
  manifestViewSwitch,
  normalizeView,
  resolveActiveView,
  scopedOptionFields,
  viewKey,
  viewOwnedFields,
  type FamilyViewEntry,
  type NativeWidgetEntry,
} from '../families/manifest.ts';
import { FAMILY_VIEWS } from './familyManifest.ts';
import type { ProjectionKey } from '../draw/laneMessages';

export { FAMILY_VIEWS };

export const NATIVE_WIDGETS: readonly NativeWidgetEntry[] = [
  { kind: 'map', renderer: 'native', load: () => import('./map') },
];

export type OptionChoicesHook = NonNullable<WidgetRenderer['useOptionChoices']>;

export const OPTION_CHOICES: Readonly<Record<string, () => Promise<OptionChoicesHook>>> = {
  'air-kills-losses': () => import('../dashboard/useMilitaryAirLedger').then(module => module.useAirLossChoices),
  'equipment-produced': () => import('./equipment-produced/useProduced').then(module => module.useProducedTypeChoices),
  'equipment-stockpile': () => import('./equipment-stockpile/useStockpileChoices').then(module => module.useStockpileChoices),
  'faction-pooled-strength': () => import('../dashboard/factionChoice').then(module => module.useFactionChoices),
  'general-career': () => import('./general-career/useLeaderChoices').then(module => module.useLeaderChoices),
};
export const OPTION_PRESETS: Readonly<Record<string, () => Promise<NonNullable<WidgetRenderer['usePresets']>>>> = {
  'idea-timeline': () => import('./idea-timeline/ideaTimelinePresets').then(module => module.useIdeaTimelinePresets),
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
  'division-log': { load: () => import('./division-log') },
  'naval-battles': { load: () => import('./naval-battles') },
  'land-battles': { load: () => import('./land-battles') },
  'ships-sunk': { load: () => import('./ships-sunk') },
  'peace-deals': { load: () => import('./peace-deals') },
  'market-ledger': { load: () => import('./market-ledger') },
  'division-template': { load: () => import('./division-template') },
  'doctrine-mastery': { load: () => import('./doctrine-mastery') },
  'construction-log': { load: () => import('./construction-log') },
  'equipment-designs': { load: () => import('./equipment-designs') },
  'equipment-produced': { load: () => import('./equipment-produced/family.tsx') },
  'espionage-log': { load: () => import('./espionage-log') },
  'event-choices': { load: () => import('./event-choices') },
  'event-feed': { load: () => import('./event-feed') },
  'factions': { load: () => import('./factions') },
  'faction-strength': { load: () => import('./faction-strength/family.tsx'), views: ['lines'] },
  'fuel': { load: () => import('./fuel') },
  'fuel-branches': { load: () => import('./fuel-branches') },
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

for (const entry of FAMILY_VIEWS) {
  let list = familyByKind.get(entry.kind);
  if (!list) familyByKind.set(entry.kind, list = []);
  list.push(entry);
  familyByPair.set(viewKey(entry.kind, entry.view), entry);
}

const EMPTY_FAMILY: readonly FamilyViewEntry[] = [];

export function familyViewsForKind(kind: string): readonly FamilyViewEntry[] {
  return familyByKind.get(kind) ?? EMPTY_FAMILY;
}

export function laneModules(kinds: Iterable<string>): { readonly families: string[]; readonly projections: ProjectionKey[] } {
  const families = new Set<string>();
  const projections = new Map<string, ProjectionKey>();
  for (const kind of new Set(kinds)) for (const entry of familyViewsForKind(kind)) {
    families.add(entry.family);
    const view = normalizeView(entry.view);
    projections.set(`${entry.family}\u0000${kind}\u0000${view}`, [entry.family, kind, view]);
  }
  return { families: [...families], projections: [...projections.values()] };
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

export function resolveWidgetActiveView(
  kind: string,
  storedOptions?: WidgetOptions,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options'>,
): string {
  const meta = metaOverride ?? widgetDefinition(kind);
  return resolveActiveView(storedOptions, meta, familyViewsForKind(kind));
}

export function resolveWidgetScopedOptions(
  kind: string,
  storedOptions?: WidgetOptions,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options'>,
): readonly OptionField[] {
  const meta = metaOverride ?? widgetDefinition(kind);
  const familyEntries = familyViewsForKind(kind);
  const activeView = resolveActiveView(storedOptions, meta, familyEntries);
  return scopedOptionFields(meta, activeView, familyEntries);
}

export function resolveWidgetViewSwitch(
  kind: string,
  metaOverride?: Pick<WidgetMeta, 'viewSwitch' | 'options' | 'title'>,
): WidgetViewSwitch | undefined {
  const meta = metaOverride ?? widgetDefinition(kind);
  return manifestViewSwitch(meta, familyViewsForKind(kind));
}

export function manifestSlotsForKind(kind: string): readonly ('render' | 'countryChip' | 'subjectMenu')[] {
  const ui = Object.hasOwn(FAMILY_VIEW_UI, kind) ? FAMILY_VIEW_UI[kind] : undefined;
  if (ui?.slots?.length) return ui.slots;
  return ['render'];
}


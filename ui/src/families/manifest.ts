import type { WidgetMeta, WidgetRenderer, WidgetViewSwitch } from '../dashboard/definitions';
import { scopedOptionKey, type OptionField, type WidgetOptions } from '../dashboard/options.ts';
import type { LaneView } from '../draw/lane.ts';
import type { FamilyProjection } from './contract.ts';
import type { FamilyHostAdapter } from './host/adapter.ts';
import type { FamilyId, FamilyViewSpec } from './plan.ts';
import { viewSwitchControl } from './settingsControls.ts';

export const DEFAULT_VIEW = 'default';

export function normalizeView(view?: string): string {
  return view && view.length > 0 ? view : DEFAULT_VIEW;
}

export function viewKey(kind: string, view?: string): string {
  return `${kind}\u0000${normalizeView(view)}`;
}

export function viewOwnedFields(fields: readonly OptionField[], view: string | undefined): readonly OptionField[] {
  const active = normalizeView(view);
  if (active === DEFAULT_VIEW) return fields;
  return fields.map(field => ((field.kind === 'choice' && field.choices.some(choice => choice.value === active)) || field.kind === 'regions'
    ? field
    : { ...field, storeAs: scopedOptionKey(field.key, active) }));
}

export interface FamilyViewModule<TModel = any, TOptions = any, TDeclaration = any> {
  readonly family: FamilyId;
  readonly kind: string;
  readonly view?: string;
  readonly projection: FamilyProjection<TModel, TOptions>;
  readonly declaration?: TDeclaration;
  readonly spec?: FamilyViewSpec<TModel, TOptions>;
}

export interface FamilyViewEntry {
  readonly kind: string;
  readonly view?: string;
  readonly label?: string;
  readonly family: FamilyId;
  readonly viewOption?: string;
  readonly options?: readonly OptionField[];
  readonly loadSpec: () => Promise<FamilyViewModule>;
}

export interface FamilyManifestModule {
  readonly family: FamilyId;
  readonly laneView: (wake: (widget: number) => void) => LaneView;
  readonly views: readonly FamilyViewEntry[];
  readonly hostAdapter?: FamilyHostAdapter<any, any, any, any>;
}

export interface LegacyWidgetEntry {
  readonly kind: string;
  readonly view?: string;
  readonly label?: string;
  readonly playsBackDated?: boolean;
  readonly slots?: readonly ('render' | 'countryChip' | 'subjectMenu')[];
  readonly load: () => Promise<{ default: WidgetRenderer }>;
}

export interface ViewOptionChoice {
  readonly value: string;
  readonly label: string;
  readonly source: 'family' | 'legacy';
  readonly family?: FamilyId;
}

export function resolveViewOptionKey(
  meta: Pick<WidgetMeta, 'viewSwitch' | 'options'> | undefined,
  familyEntries: readonly FamilyViewEntry[],
): string {
  for (const entry of familyEntries) {
    if (entry.viewOption) return entry.viewOption;
  }
  if (meta?.viewSwitch?.option) return meta.viewSwitch.option;
  const viewField = meta?.options?.find(field => field.key === 'view' && field.kind === 'choice');
  if (viewField) return viewField.key;
  return 'view';
}

export function declaredViewChoices(
  meta: Pick<WidgetMeta, 'viewSwitch' | 'options'> | undefined,
  familyEntries: readonly FamilyViewEntry[],
  legacyEntries: readonly LegacyWidgetEntry[],
): readonly ViewOptionChoice[] {
  const out: ViewOptionChoice[] = [];
  const seen = new Set<string>();
  const metaFieldKey = resolveViewOptionKey(meta, familyEntries);
  const metaChoiceField = meta?.options?.find(
    (field): field is Extract<OptionField, { kind: 'choice' }> => field.key === metaFieldKey && field.kind === 'choice',
  );
  const labelFor = (view: string, explicit?: string) =>
    explicit ?? metaChoiceField?.choices.find(choice => choice.value === view)?.label ?? view;

  for (const entry of familyEntries) {
    const view = normalizeView(entry.view);
    if (seen.has(view)) continue;
    seen.add(view);
    out.push({ value: view, label: labelFor(view, entry.label), source: 'family', family: entry.family });
  }
  for (const entry of legacyEntries) {
    const view = normalizeView(entry.view);
    if (seen.has(view)) continue;
    seen.add(view);
    out.push({ value: view, label: labelFor(view, entry.label), source: 'legacy' });
  }
  const declared = metaChoiceField?.choices.map(choice => choice.value);
  if (!declared) return out;
  const rank = (view: string) => {
    const at = declared.indexOf(view);
    return at < 0 ? declared.length : at;
  };
  return out.map((choice, index) => ({ choice, index }))
    .sort((a, b) => rank(a.choice.value) - rank(b.choice.value) || a.index - b.index)
    .map(item => item.choice);
}

export function resolveActiveView(
  stored: WidgetOptions | undefined,
  meta: Pick<WidgetMeta, 'viewSwitch' | 'options'> | undefined,
  familyEntries: readonly FamilyViewEntry[],
  legacyEntries: readonly LegacyWidgetEntry[],
): string {
  const views = declaredViewChoices(meta, familyEntries, legacyEntries);
  if (views.length <= 1) return views[0]?.value ?? DEFAULT_VIEW;
  const optionKey = resolveViewOptionKey(meta, familyEntries);
  const raw = stored?.[optionKey];
  if (typeof raw === 'string' && views.some(item => item.value === raw)) return raw;
  const field = meta?.options?.find(item => item.key === optionKey);
  if (field?.kind === 'choice' && views.some(item => item.value === field.default)) return field.default;
  return views[0]!.value;
}

export function scopedOptionFields(
  meta: Pick<WidgetMeta, 'viewSwitch' | 'options'> | undefined,
  activeView: string,
  familyEntries: readonly FamilyViewEntry[],
  legacyEntries: readonly LegacyWidgetEntry[],
): readonly OptionField[] {
  const activeFamily = familyEntries.find(entry => normalizeView(entry.view) === activeView);
  const views = declaredViewChoices(meta, familyEntries, legacyEntries);
  const optionKey = resolveViewOptionKey(meta, familyEntries);
  const baseFields = meta?.options ?? [];

  if (!activeFamily) return baseFields;
  if (!activeFamily.options) return viewOwnedFields(baseFields, activeView);

  const result: OptionField[] = [];
  const seen = new Set<string>();

  if (views.length > 1) {
    const existingViewField = baseFields.find(field => field.key === optionKey);
    const viewField = existingViewField ?? viewSwitchControl(
      views.map(item => ({ value: item.value, label: item.label })),
      views[0]!.value,
      { key: optionKey },
    );
    result.push(viewField);
    seen.add(viewField.key);
  }

  for (const field of viewOwnedFields(activeFamily.options, activeView)) {
    if (seen.has(field.key)) continue;
    seen.add(field.key);
    result.push(field);
  }

  return result;
}

export function manifestViewSwitch(
  meta: Pick<WidgetMeta, 'viewSwitch' | 'options' | 'title'> | undefined,
  familyEntries: readonly FamilyViewEntry[],
  legacyEntries: readonly LegacyWidgetEntry[],
): WidgetViewSwitch | undefined {
  if (meta?.viewSwitch) return meta.viewSwitch;
  const views = declaredViewChoices(meta, familyEntries, legacyEntries);
  if (views.length <= 1) return undefined;
  const option = resolveViewOptionKey(meta, familyEntries);
  return {
    option,
    choices: views.map(item => ({ value: item.value, label: item.label })),
  };
}

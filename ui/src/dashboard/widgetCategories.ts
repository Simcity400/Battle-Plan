// Pure grouping and search for the widget catalogue; no React.
import type { IconName } from '../components/Icons';
import { PAGES } from './pages.ts';
import { resolveOptions } from './options.ts';
import type { OptionField, WidgetOptions } from './options';

interface Categorised { kind: string; title: string; description?: string; category?: string }
/** `page` is the Analytics page a group shares its name, icon and colour with. */
export interface WidgetGroup<T> { label: string; page?: string; icon?: IconName; items: T[] }

/** A widget's declared category, else the first subject page that shows it, else Overview's.
 * Groups follow the sidebar's page order; a declared category no page names comes after them, then Other. */
export function widgetGroups<T extends Categorised>(definitions: readonly T[]): WidgetGroup<T>[] {
  const subjects = PAGES.filter(page => page.id !== 'overview');
  const pageLabel = (kind: string) => (subjects.find(page => page.panels.some(([panel]) => panel === kind))
    ?? PAGES.find(page => page.panels.some(([panel]) => panel === kind)))?.label;
  const groups = new Map<string, WidgetGroup<T>>(PAGES.map(page => [page.label, { label: page.label, page: page.id, icon: page.icon, items: [] }]));
  for (const definition of definitions) {
    const label = definition.category || pageLabel(definition.kind) || 'Other';
    if (!groups.has(label)) groups.set(label, { label, items: [] });
    groups.get(label)!.items.push(definition);
  }
  const other = groups.get('Other');
  if (other) { groups.delete('Other'); groups.set('Other', other); }
  return [...groups.values()].filter(group => group.items.length);
}

/** Every word of the query appears in the widget's title, description or category, ignoring case. */
export function matchesWidget(query: string, ...texts: (string | undefined)[]): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const text = texts.filter(Boolean).join(' ').toLowerCase();
  return words.every(word => text.includes(word));
}

type Options = Readonly<Record<string, unknown>>;
interface Preset { key: string; initial: { title?: string; options?: Options } }
interface Presets { presets: readonly Preset[]; reason: string | null }
interface Catalogued extends Categorised { options?: readonly OptionField[]; variants?: readonly Preset[] }
/** A catalogue card: a widget, one of its views (`variant`), or one of its content presets, with the
 * title and options it starts with. `match` holds the option values a placed copy has when it
 * shows this card's view. `note` says why a content-derived widget has no presets yet. */
export interface CatalogueEntry<T> {
  key: string; definition: T; group: WidgetGroup<T>; title: string;
  initial?: { title?: string; options?: Options }; match: Options; variant: boolean; note: string | null;
}

/** Each group's cards: a widget's own (or one per content preset in its place while it has any),
 * then one per view it lists as a variant. */
export function catalogueEntries<T extends Catalogued>(groups: readonly WidgetGroup<T>[], presets: ReadonlyMap<string, Presets>): CatalogueEntry<T>[][] {
  return groups.map(group => group.items.flatMap(definition => {
    const entry = (preset: Preset, variant: boolean) => ({ key: `${definition.kind}:${preset.key}`, definition, group,
      title: preset.initial.title ?? definition.title, initial: preset.initial, match: preset.initial.options ?? {}, variant, note: null });
    const own = presets.get(definition.kind);
    const variants = (definition.variants ?? []).map(preset => entry(preset, true));
    if (own?.presets.length) return [...own.presets.map(preset => entry(preset, false)), ...variants];
    // The widget's own card shows the default of every option its views change.
    const keys = new Set(variants.flatMap(item => Object.keys(item.match)));
    const match = Object.fromEntries((definition.options ?? []).filter(field => keys.has(field.key)).map(field => [field.key, field.default]));
    return [{ key: definition.kind, definition, group, title: definition.title, match, variant: false, note: own?.reason ?? null }, ...variants];
  }));
}

/** How many of the page's widgets show a card's view: its kind with the option values it matches,
 * a stored value read as the field reads it (a value equal to the default is not stored). */
export function placedCount(entry: { definition: { kind: string; options?: readonly OptionField[] }; match: Options },
  placed: readonly { kind: string; options?: Options }[]): number {
  return placed.filter(widget => {
    if (widget.kind !== entry.definition.kind) return false;
    const resolved = resolveOptions(entry.definition.options, widget.options as WidgetOptions | undefined);
    return Object.entries(entry.match).every(([key, value]) => (Object.hasOwn(resolved, key) ? resolved[key] : widget.options?.[key]) === value);
  }).length;
}

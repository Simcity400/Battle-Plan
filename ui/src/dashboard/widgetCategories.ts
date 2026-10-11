// Pure grouping and search for the widget catalogue; no React.
import type { IconName } from '../components/Icons';
import { PAGES } from './pages.ts';

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
interface Presets { presets: readonly { key: string; initial: { title?: string; options?: Options } }[]; reason: string | null }
/** A catalogue card: a widget, or one of its content presets with the title and options it starts with.
 * `note` says why a content-derived widget has no presets yet. */
export interface CatalogueEntry<T> {
  key: string; definition: T; group: WidgetGroup<T>; title: string;
  initial?: { title?: string; options?: Options }; note: string | null;
}

/** Each group's cards: a widget's own, or one per content preset in its place while it has any. */
export function catalogueEntries<T extends Categorised>(groups: readonly WidgetGroup<T>[], presets: ReadonlyMap<string, Presets>): CatalogueEntry<T>[][] {
  return groups.map(group => group.items.flatMap(definition => {
    const own = presets.get(definition.kind);
    if (own?.presets.length) return own.presets.map(preset => ({ key: `${definition.kind}:${preset.key}`, definition, group,
      title: preset.initial.title ?? definition.title, initial: preset.initial, note: null }));
    return [{ key: definition.kind, definition, group, title: definition.title, note: own?.reason ?? null }];
  }));
}

/** How many of the page's widgets match a card: its kind and, for a preset, the options it chose. */
export function placedCount(entry: { definition: { kind: string }; initial?: { options?: Options } },
  placed: readonly { kind: string; options?: Options }[]): number {
  return placed.filter(widget => widget.kind === entry.definition.kind
    && Object.entries(entry.initial?.options ?? {}).every(([key, value]) => widget.options?.[key] === value)).length;
}

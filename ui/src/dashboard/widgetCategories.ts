import type { IconName } from '../components/Icons';
import { PAGES } from './pages.ts';
import type { Page } from './pages.ts';
import type { OptionField, OptionValue } from './options';

type Options = Readonly<Record<string, unknown>>;
interface Preset { key: string; initial: { title?: string; options?: Options }; category?: string }
interface Presets { presets: readonly Preset[]; reason: string | null; detail?: string }
interface Categorised { kind: string; title: string; description?: string; category?: string; variants?: readonly Preset[] }
export interface WidgetGroup<T> { label: string; page?: string; icon?: IconName; items: T[] }

export function widgetGroups<T extends Categorised>(definitions: readonly T[], pages: readonly Page[] = PAGES): WidgetGroup<T>[] {
  const subjects = pages.filter(page => page.id !== 'overview');
  const pageLabel = (kind: string) => (subjects.find(page => page.panels.some(([panel]) => panel === kind))
    ?? pages.find(page => page.panels.some(([panel]) => panel === kind)))?.label;
  const groups = new Map<string, WidgetGroup<T>>(pages.map(page => [page.label, { label: page.label, page: page.id, icon: page.icon, items: [] }]));
  const used = new Set<string>();
  const ensure = (label: string) => {
    if (!groups.has(label)) groups.set(label, { label, items: [] });
    used.add(label);
    return groups.get(label)!;
  };
  for (const definition of definitions) {
    const label = definition.category || pageLabel(definition.kind) || 'Other';
    ensure(label).items.push(definition);
    for (const variant of definition.variants ?? []) if (variant.category) ensure(variant.category);
  }
  const other = groups.get('Other');
  if (other) { groups.delete('Other'); groups.set('Other', other); }
  return [...groups.values()].filter(group => used.has(group.label));
}

export function matchesWidget(query: string, ...texts: (string | undefined)[]): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const text = texts.filter(Boolean).join(' ').toLowerCase();
  return words.every(word => text.includes(word));
}

interface Catalogued extends Categorised { options?: readonly OptionField[] }
export interface CatalogueEntry<T> {
  key: string; definition: T; group: WidgetGroup<T>; title: string;
  initial?: { title?: string; options?: Options }; match: Options; variant: boolean; note: string | null; detail?: string;
}

export function catalogueEntries<T extends Catalogued>(groups: readonly WidgetGroup<T>[], presets: ReadonlyMap<string, Presets>): CatalogueEntry<T>[][] {
  const out: CatalogueEntry<T>[][] = groups.map(() => []);
  const byLabel = new Map(groups.map((group, index) => [group.label, index]));
  groups.forEach((group, groupIndex) => {
    for (const definition of group.items) {
      const push = (entry: CatalogueEntry<T>, category?: string) => {
        const index = (category ? byLabel.get(category) : undefined) ?? groupIndex;
        out[index]!.push(index === groupIndex ? entry : { ...entry, group: groups[index]! });
      };
      const make = (preset: Preset, variant: boolean): readonly [CatalogueEntry<T>, string | undefined] => [
        { key: `${definition.kind}:${preset.key}`, definition, group,
          title: preset.initial.title ?? definition.title, initial: preset.initial, match: preset.initial.options ?? {}, variant, note: null },
        preset.category,
      ];
      const own = presets.get(definition.kind);
      const variants = (definition.variants ?? []).map(preset => make(preset, true));
      if (own?.presets.length) {
        for (const preset of own.presets) { const [entry, cat] = make(preset, false); push(entry, cat); }
        for (const [entry, cat] of variants) push(entry, cat);
        continue;
      }
      const keys = new Set(variants.flatMap(([item]) => Object.keys(item.match)));
      const match = Object.fromEntries((definition.options ?? []).filter(field => keys.has(field.key)).map(field => [field.key, field.default]));
      push({ key: definition.kind, definition, group, title: definition.title, match, variant: false, note: own?.reason ?? null, detail: own?.detail });
      for (const [entry, cat] of variants) push(entry, cat);
    }
  });
  return out;
}

export function placedCount(entry: { definition: { kind: string }; match: Options },
  placed: readonly { kind: string; options: Readonly<Record<string, OptionValue>> }[]): number {
  return placed.filter(widget => widget.kind === entry.definition.kind
    && Object.entries(entry.match).every(([key, value]) => widget.options[key] === value)).length;
}

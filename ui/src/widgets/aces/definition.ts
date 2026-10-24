import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { NARROW_TABLE_OPTION, tableSortOptions } from '../../charts/table/options.ts';
export const DEFAULT_SORT = { column: 'date', direction: 'descending' } as const;
export const SORT_CHOICES = [
  { key: 'date', label: 'Newest first', direction: 'descending' },
  { key: 'name', label: 'Name', direction: 'ascending' },
] as const;
// No numerical bars or rank column in this card; keep only its approved sort.
export const OPTIONS: readonly OptionField[] = [...tableSortOptions(SORT_CHOICES, DEFAULT_SORT), NARROW_TABLE_OPTION];
export const definition = {
  kind: 'aces', version: 1, title: 'Aces', icon: 'star',
  description: 'Every country’s aces, their recorded wing and aircraft, and bounded first-sighting and death dates.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
  availability: { product: 'air-aces', params: { view: 'catalogue' } },
} as const satisfies WidgetMeta;

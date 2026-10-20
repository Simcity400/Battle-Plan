import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const COLUMNS = [
  { key: 'exile', label: 'Exile' }, { key: 'host', label: 'Host' }, { key: 'legitimacy', label: 'Legitimacy' },
  { key: 'manpower', label: 'Manpower' }, { key: 'air', label: 'Air crews' }, { key: 'leaders', label: 'Army leaders' },
] as const;
export const DEFAULT_SORT = { column: 'manpower', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT)];
export const definition = {
  kind: 'governments-in-exile', version: 1, title: 'Governments in exile', category: 'Politics', icon: 'flag',
  description: 'Every government in exile: its host, legitimacy, manpower and leaders.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'country', label: 'Country', as: 'flagName', offered: OFFERED.country },
  { key: 'host', label: 'Host', as: 'flag', offered: OFFERED.country },
  { key: 'legitimacy', label: 'Legitimacy', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'manpower', label: 'Manpower', as: 'faint', offered: OFFERED.number },
  { key: 'air', label: 'Air crews', as: 'faint', offered: OFFERED.number },
  { key: 'leaders', label: 'Army leaders', as: 'faint', offered: OFFERED.number },
];
export const DEFAULT_SORT = { column: 'manpower', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', overflow: 'scroll', sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'governments-in-exile', version: 1, title: 'Governments in exile', category: 'Politics', icon: 'flag',
  description: 'Every government in exile: its host, legitimacy, manpower and leaders.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

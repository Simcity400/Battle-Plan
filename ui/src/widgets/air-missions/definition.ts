import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'region', label: 'Region', as: 'plain', offered: OFFERED.text },
];
export const WINGS_COLUMN: TableColumnDefaults = { key: 'wings', label: 'Wings', as: 'faint', tone: 'grey', offered: OFFERED.number };
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'tooltip', sort: { column: 'wings', direction: 'descending' } };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'air-missions', version: 1, title: 'Air missions', icon: 'chart-line', category: 'Air',
  description: 'Where a country’s air wings fly and on which missions, region by region.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

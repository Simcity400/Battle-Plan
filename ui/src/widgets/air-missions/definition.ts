import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const REGION_COLUMN: TableColumnDefaults = { key: 'region', label: 'Region', as: 'plain', offered: OFFERED.text };
export const WINGS_COLUMN: TableColumnDefaults = { key: 'wings', label: 'Wings', as: 'faint', tone: 'grey', offered: OFFERED.number };
export const COLUMNS: readonly TableColumnDefaults[] = [REGION_COLUMN, WINGS_COLUMN];
export const RECORDED_MISSION_BITS = 18;
export const missionKey = (bit: string | number) => `m${bit}`;
export const missionDefaults = (bit: string | number, label: string): TableColumnDefaults => ({ key: missionKey(bit), label, as: 'faint', tone: 'grey', offered: OFFERED.number });
export const SETTINGS_COLUMNS: readonly TableColumnDefaults[] = [
  REGION_COLUMN,
  ...Array.from({ length: RECORDED_MISSION_BITS }, (_, bit) => missionDefaults(bit, `Air mission ${bit + 1}`)),
  WINGS_COLUMN,
];
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', sort: { column: 'wings', direction: 'descending' } };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'air-missions', version: 1, title: 'Air missions', icon: 'chart-line', category: 'Air',
  description: 'Where a country’s air wings fly and on which missions, region by region.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

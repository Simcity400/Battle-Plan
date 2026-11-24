import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { REACHED_COUNTRY_OPTION } from '../../dashboard/reachedCountry.ts';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'division', label: 'Division', as: 'icon', offered: ['icon', 'plain'] },
  { key: 'training', label: 'Training', as: 'meterBefore', tone: 'blue', offered: OFFERED.number },
  { key: 'equipment', label: 'Equipment', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'manpower', label: 'Manpower', as: 'plain', offered: ['plain'] },
];
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', sort: { column: 'training', direction: 'descending' } };
export const OPTIONS: readonly OptionField[] = [REACHED_COUNTRY_OPTION, ...tableFamilyOptions(COLUMNS, TABLE_BASE)];
export const definition = {
  kind: 'training-queue', version: 1, title: 'Training queue', icon: 'helmet', category: 'Army',
  description: 'Divisions in training under each deployment line, with training, equipment and manpower.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  countries: { max: 1 }, minSize: { w: 4, h: 5 }, defaultSize: { w: 7, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'name', label: 'Refit to', as: 'plain', offered: OFFERED.text },
  { key: 'progress', label: 'Progress', as: 'meterBefore', tone: 'blue', offered: OFFERED.number },
  { key: 'strength', label: 'Strength', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'dockyards', label: 'Dockyards', as: 'faint', tone: 'grey', offered: OFFERED.number },
];
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'tooltip', sort: { column: 'progress', direction: 'descending' } };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'refits', version: 1, title: 'Refits', icon: 'cube', category: 'Navy',
  description: 'Refit lines at work, their progress and the ships on them.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

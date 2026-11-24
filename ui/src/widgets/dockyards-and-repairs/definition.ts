import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'name', label: 'Port', as: 'plain', offered: OFFERED.text },
  { key: 'strength', label: 'Strength', as: 'meterBefore', tone: 'grey', offered: OFFERED.number },
  { key: 'priority', label: 'Repair priority', as: 'plain', offered: ['plain'] },
  { key: 'dockyards', label: 'Dockyards', as: 'faint', tone: 'grey', offered: OFFERED.number },
];
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', sort: { column: 'priority', direction: 'descending' } };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'dockyards-and-repairs', version: 1, title: 'Dockyards & repairs', icon: 'anchor', category: 'Navy',
  description: 'Each port’s repair dockyards and the ships queued there, with their strength.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

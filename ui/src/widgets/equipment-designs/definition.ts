import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'kind', label: '', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'design', label: 'Design', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'type', label: 'Equipment', as: 'plain', offered: OFFERED.text },
  { key: 'state', label: 'State', as: 'dot', offered: OFFERED.status },
  { key: 'view', label: '', as: 'plain', offered: OFFERED.text, hideable: false },
];
export const DEFAULT_SORT = { column: 'state', direction: 'ascending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);

export const definition = {
  kind: 'equipment-designs', version: 1, title: 'Equipment designs', icon: 'layers', category: 'Economy',
  description: 'A country’s saved equipment designs, current and obsolete.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: 1 }, minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

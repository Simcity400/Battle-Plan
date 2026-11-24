import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'organisation', label: 'Organisation', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'size', label: 'Size', as: 'faint', offered: OFFERED.number },
  { key: 'funds', label: 'Funds', as: 'faint', offered: OFFERED.number },
  { key: 'traits', label: 'Traits', as: 'faint', offered: OFFERED.number },
  { key: 'policy', label: 'Policy', as: 'plain', offered: OFFERED.text },
  { key: 'view', label: '', as: 'plain', offered: OFFERED.text, hideable: false },
];
export const DEFAULT_SORT = { column: 'size', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);

export const definition = {
  kind: 'industrial-organisations', version: 1, title: 'Industrial organisations', icon: 'factory', category: 'Economy',
  description: 'A country’s industrial organisations: size, funds, traits and policy.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: 1 }, minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'country', label: 'Country', as: 'flagName', offered: OFFERED.country, showNamesDefault: true, hideable: false },
  { key: 'term', label: 'Term served', as: 'track', tone: 'grey', offered: OFFERED.number },
  { key: 'days', label: 'Days left', as: 'plain', offered: OFFERED.number },
  { key: 'election', label: 'Election', as: 'plain', offered: OFFERED.number },
];
export const DEFAULT_SORT = { column: 'days', direction: 'ascending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', showRank: false, sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'next-elections', version: 1, title: 'Next elections', category: 'Politics', icon: 'ballot',
  description: 'Countries that hold elections: how far through the term each is, and when it votes next.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

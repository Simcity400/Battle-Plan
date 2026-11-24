import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'country', label: 'Country', as: 'flagName', offered: OFFERED.country, showNamesDefault: true, hideable: false },
  { key: 'gained', label: 'States gained', as: 'faint', offered: OFFERED.number },
  { key: 'lost', label: 'States lost', as: 'faint', offered: OFFERED.number },
  { key: 'net', label: 'Net', as: 'plain', offered: OFFERED.number },
  { key: 'victoryPoints', label: 'Victory points', as: 'plain', offered: OFFERED.number },
];
export const DEFAULT_SORT = { column: 'net', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);

export const definition = {
  kind: 'territory-transfers', version: 3, title: 'Territory transfers', category: 'Diplomacy', icon: 'flows',
  description: 'Who gained and lost states in peace settlements.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' }, minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

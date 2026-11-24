import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'kind', label: 'Division type', as: 'flag', offered: ['flag'] },
  { key: 'country', label: 'Country', as: 'flag', offered: OFFERED.country },
  { key: 'division', label: 'Division', as: 'plain', offered: OFFERED.text },
  { key: 'medals', label: 'Medals', as: 'faint', tone: 'grey', offered: OFFERED.number },
  { key: 'experience', label: 'Experience', as: 'plain', offered: ['plain'] },
  { key: 'kills', label: 'Casualties taken', as: 'faint', tone: 'grey', offered: OFFERED.number },
];
export const DEFAULT_SORT = { column: 'medals', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'tooltip', sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'most-decorated-divisions', version: 1, title: 'Most decorated divisions', icon: 'star', category: 'Army',
  description: 'Every country’s most decorated divisions: medals, experience and casualties taken.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOf, tableSortOptions } from '../../charts/table/options.ts';
import { TOP_COUNTRIES } from '../../charts/race/options.ts';
import { tableFamilyOptions, type TableBase } from '../../families/table/settings.ts';

export const DEFAULT_SORT = { column: '', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'comfortable', overflow: 'tooltip', sort: { column: 'country', direction: 'ascending' } };
export const OPTIONS: readonly OptionField[] = [
  { key: 'columns', section: 'options', label: 'Columns', kind: 'content-set', default: '',
    automatic: 'Default columns', hint: 'Columns appear in the order you choose.' },
  ...TABLE_OPTIONS.filter(field => field.key === 'rankMarkers'), ...tableFamilyOptions([], TABLE_BASE), ...tableSortOptions([], DEFAULT_SORT, true),
  { key: 'topMeasure', section: 'countries', label: 'Top N by', kind: 'content-choice', default: '', automatic: 'Default measure',
    hint: 'Ranks countries before sorting by name.',
    visibleWhen: (options, countries) => tableSortOf(options.tableSort, DEFAULT_SORT).column === 'country'
      && countries?.kind === 'group' && countries.group.type === 'top' },
];

export const definition = {
  kind: 'scoreboard', version: 1, title: 'Scoreboard', icon: 'grid',
  description: 'Compare countries across selected measures.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

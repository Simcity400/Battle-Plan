import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOf, tableSortOptions } from '../../charts/table/options.ts';
import { TOP_COUNTRIES } from '../../charts/race/options.ts';

/** The recorded registry supplies the default measure; no second measure catalogue. */
export const DEFAULT_SORT = { column: '', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'columns', section: 'options', label: 'Columns', kind: 'content-set', default: '', max: Number.MAX_SAFE_INTEGER,
    automatic: 'Default columns', hint: 'The registry supplies the defaults. Columns follow the order you choose. A dash means the value is unavailable, never zero.' },
  ...TABLE_OPTIONS, ...tableSortOptions([], DEFAULT_SORT, true),
  { key: 'topMeasure', section: 'countries', label: 'Top N by', kind: 'content-choice', default: '', automatic: 'Default measure',
    hint: 'Country sorting alphabetises the countries ranked by this measure.',
    visibleWhen: (options, countries) => tableSortOf(options.tableSort, DEFAULT_SORT).column === 'country'
      && countries?.kind === 'group' && countries.group.type === 'top' },
];

export const definition = {
  kind: 'scoreboard', version: 1, title: 'Scoreboard', icon: 'grid',
  description: 'Compare exact recorded measures across countries, with columns chosen in settings.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

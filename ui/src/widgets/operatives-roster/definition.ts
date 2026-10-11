import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { CountrySelection, WidgetTime } from '../../dashboard/model';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const COLUMNS = [
  { key: 'name', label: 'Operative' }, { key: 'skill', label: 'Skill' },
  { key: 'experience', label: 'Experience' }, { key: 'assignment', label: 'Assignment' },
  { key: 'captured', label: 'Captured' },
] as const;
export const DEFAULT_SORT = { column: 'name', direction: 'ascending' } as const;
const departedVisible = (_options: Readonly<Record<string, OptionValue>>, _countries?: CountrySelection, mode?: WidgetTime['mode']) => mode === 'campaign';
export const OPTIONS: readonly OptionField[] = [
  { key: 'departed', section: 'options', label: 'Include departed operatives', kind: 'toggle', default: true, visibleWhen: departedVisible },
  { key: 'showRank', section: 'options', label: 'Show rank', kind: 'toggle', default: true },
  ...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT),
];
export const definition = {
  kind: 'operatives-roster', version: 1, title: 'Operatives roster', icon: 'shield',
  description: 'Operative names, skill, experience, assignment and capture status, with a record of departures.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER }, minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

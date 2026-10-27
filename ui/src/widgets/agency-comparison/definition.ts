import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const COLUMNS = [
  { key: 'country', label: 'Country' }, { key: 'agency', label: 'Agency' },
  { key: 'upgrades', label: 'Agency upgrades' }, { key: 'defense', label: 'Defence' },
  { key: 'active', label: 'Operatives active / maximum' },
] as const;
export const DEFAULT_SORT = { column: 'defense', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'includeUncreated', section: 'options', label: 'Include agencies not created', kind: 'toggle', default: false },
  { key: 'intelRecords', section: 'options', label: 'Show recorded intel records', hint: 'The raw intel the recording holds for one viewing country.', kind: 'toggle', default: false },
  { key: 'topMeasure', section: 'options', label: 'Top countries by', hint: 'Ranks Top N independently of table sorting.', kind: 'choice', default: 'defense', visibleWhen: (_options, countries) => countries?.kind === 'group' && countries.group.type === 'top',
    choices: [{ value: 'defense', label: 'Defence' }, { value: 'active', label: 'Active operatives' }, { value: 'maximum', label: 'Maximum operatives' }] },
  ...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT),
];
export const definition = {
  kind: 'agency-comparison', version: 1, title: 'Agency comparison', icon: 'shield',
  description: 'Compare countries’ agencies, defence and operatives.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true, ranked: true }, minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

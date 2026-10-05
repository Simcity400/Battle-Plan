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
  { key: 'includeUncreated', section: 'options', label: 'Include agencies not created', hint: 'Show the game’s localised default agency name and recorded values for countries without a created agency', kind: 'toggle', default: false },
  { key: 'topMeasure', section: 'options', label: 'Top N by', hint: 'The ranking for a Top N country group, independent of the table’s visible sort', kind: 'choice', default: 'defense',
    choices: [{ value: 'defense', label: 'Defence' }, { value: 'active', label: 'Active operatives' }, { value: 'maximum', label: 'Maximum operatives' }] },
  ...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT),
];
export const definition = {
  kind: 'agency-comparison', version: 1, title: 'Agency comparison', icon: 'shield',
  description: 'Every country’s agency, defence and operatives, ranked from recorded agency-screen values.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true, ranked: true }, minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const COLUMNS = [
  { key: 'state', label: 'State' }, { key: 'owner', label: 'Owner' }, { key: 'occupier', label: 'Occupier' },
  { key: 'resistance', label: 'Resistance' }, { key: 'compliance', label: 'Compliance' },
  { key: 'garrison', label: 'Garrison' }, { key: 'law', label: 'Occupation law' },
] as const;
export const DEFAULT_SORT = { column: 'resistance', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'countryRole', section: 'options', label: 'Filter by', kind: 'choice', default: 'occupier',
    choices: [{ value: 'occupier', label: 'Occupier' }, { value: 'occupied', label: 'Occupied country' }],
    hint: 'Occupied matches the original occupied country, which can differ from the legal owner.' },
  ...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT),
];
export const definition = {
  kind: 'occupied-states', version: 1, title: 'Occupied states', icon: 'globe',
  description: 'Occupied states, resistance, compliance, garrisons and occupation laws.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER }, minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

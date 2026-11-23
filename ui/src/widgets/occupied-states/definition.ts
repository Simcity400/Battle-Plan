import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'state', label: 'State', as: 'plain', offered: OFFERED.text },
  { key: 'owner', label: 'Owner', as: 'flagName', offered: OFFERED.country },
  { key: 'occupier', label: 'Occupier', as: 'flagName', offered: OFFERED.country },
  { key: 'resistance', label: 'Resistance', as: 'meterAfter', tone: 'red', offered: OFFERED.number },
  { key: 'compliance', label: 'Compliance', as: 'meterAfter', tone: 'blue', offered: OFFERED.number },
  { key: 'garrison', label: 'Garrison', as: 'faint', tone: 'blue', offered: OFFERED.number },
  { key: 'law', label: 'Occupation law', as: 'dot', offered: OFFERED.status },
];
export const DEFAULT_SORT = { column: 'resistance', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'comfortable', overflow: 'tooltip', sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = [
  { key: 'countryRole', section: 'options', label: 'Filter by', kind: 'choice', default: 'occupier',
    choices: [{ value: 'occupier', label: 'Occupier' }, { value: 'occupied', label: 'Occupied country' }],
    hint: 'Occupied matches the original occupied country, which can differ from the legal owner.' },
  ...tableFamilyOptions(COLUMNS, TABLE_BASE),
];
export const definition = {
  kind: 'occupied-states', version: 1, title: 'Occupied states', icon: 'globe',
  description: 'Occupied states, resistance, compliance, garrisons and occupation laws.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER }, minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 }, options: OPTIONS, playsBackDated: true,
} as const satisfies WidgetMeta;

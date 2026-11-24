import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'country', label: 'Country', as: 'flag', offered: OFFERED.country, showNamesDefault: false },
  { key: 'agency', label: 'Agency', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'upgrades', label: 'Upgrades', as: 'faint', offered: OFFERED.number },
  { key: 'defense', label: 'Counter intelligence', as: 'plain', offered: OFFERED.number },
  { key: 'operatives', label: 'Operatives', as: 'plain', offered: OFFERED.number },
  { key: 'view', label: '', as: 'plain', offered: OFFERED.text, hideable: false },
];
export const DEFAULT_SORT = { column: 'defense', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = [
  { key: 'includeUncreated', section: 'options', label: 'Include agencies not created', kind: 'toggle', default: false },
  { key: 'topMeasure', section: 'options', label: 'Top countries by', kind: 'choice', default: 'defense', visibleWhen: (_options, countries) => countries?.kind === 'group' && countries.group.type === 'top',
    choices: [{ value: 'defense', label: 'Counter intelligence' }, { value: 'active', label: 'Active operatives' }, { value: 'maximum', label: 'Maximum operatives' }] },
  ...tableFamilyOptions(COLUMNS, TABLE_BASE),
];
export const definition = {
  kind: 'agency-comparison', version: 1, title: 'Agency comparison', icon: 'shield',
  description: 'Compare countries’ agencies, counter intelligence and operatives.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true, ranked: true }, minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

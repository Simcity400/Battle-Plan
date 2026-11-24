import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { REACHED_COUNTRY_OPTION } from '../../dashboard/reachedCountry.ts';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const COLUMNS = [
  { key: 'template', label: 'Template' }, { key: 'fielded', label: 'Fielded' },
  { key: 'battalions', label: 'Battalions' }, { key: 'width', label: 'Base combat width' }, { key: 'priority', label: 'Priority' },
] as const;
export const DEFAULT_SORT = { column: 'template', direction: 'ascending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: DEFAULT_SORT };
const COLUMN_DEFAULTS: readonly TableColumnDefaults[] = COLUMNS.map(column => ({ key: column.key, label: column.label, as: 'plain', offered: OFFERED.text }));
export const COUNT_FOR_OPTION: OptionField = { key: 'countFor', section: 'options', label: 'Expeditionary forces count for', kind: 'choice', default: 'owner',
  choices: [{ value: 'owner', label: 'Owner' }, { value: 'host', label: 'Host' }] };
export const OPTIONS: readonly OptionField[] = [
  REACHED_COUNTRY_OPTION, COUNT_FOR_OPTION, ...tableFamilyOptions(COLUMN_DEFAULTS, TABLE_BASE), DRILL_OPTION,
];
export const definition = {
  kind: 'division-template', version: 1, title: 'Division template', icon: 'grid', category: 'Army',
  description: 'A country’s current division templates and how many are fielded; each opens in the game’s division designer.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: 1 }, minSize: { w: 4, h: 5 }, defaultSize: { w: 7, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

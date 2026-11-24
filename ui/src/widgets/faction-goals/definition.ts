import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'goal', label: 'Goal', as: 'icon', offered: ['icon', 'plain'] },
  { key: 'progress', label: 'Progress', as: 'meterBefore', tone: 'blue', offered: OFFERED.number },
  { key: 'state', label: 'State', as: 'dot', offered: ['dot'] },
];
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'tooltip', sort: { column: 'progress', direction: 'descending' } };

export const OPTIONS: readonly OptionField[] = [
  { key: 'goals', section: 'options', label: 'Goals', kind: 'choice', default: 'all',
    choices: [{ value: 'all', label: 'All' }, { value: 'inProgress', label: 'In progress' }, { value: 'completed', label: 'Completed' }] },
  ...tableFamilyOptions(COLUMNS, TABLE_BASE),
];

export const definition = {
  kind: 'faction-goals', version: 1, title: 'Faction goals', icon: 'target', category: 'Diplomacy',
  description: 'The goals of a country’s faction: done, in progress and cancelled.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 10 }, countries: { max: 1 }, options: OPTIONS,
} as const satisfies WidgetMeta;

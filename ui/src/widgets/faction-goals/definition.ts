import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { FACTION_OPTION } from '../../dashboard/factionOption.ts';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const COLUMNS = [{ key: 'goal', label: 'Goal' }, { key: 'progress', label: 'Progress' }, { key: 'status', label: 'Status' }] as const;
export const DEFAULT_SORT = { column: 'progress', direction: 'descending' } as const;

export const OPTIONS: readonly OptionField[] = [
  FACTION_OPTION,
  { key: 'goals', section: 'options', label: 'Goals', kind: 'choice', default: 'all',
    choices: [{ value: 'all', label: 'All' }, { value: 'inProgress', label: 'In progress' }, { value: 'completed', label: 'Completed' }] },
  ...TABLE_OPTIONS.filter(field => field.key !== 'rankMarkers'), ...tableSortOptions(COLUMNS, DEFAULT_SORT),
];

export const definition = {
  kind: 'faction-goals', version: 1, title: 'Faction goals', icon: 'target', category: 'Diplomacy',
  description: 'A faction’s shared goals: done, in progress and cancelled.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 10 }, countries: { max: 1 }, options: OPTIONS,
} as const satisfies WidgetMeta;

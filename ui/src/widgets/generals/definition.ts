import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { GROW_WITH_PLAYBACK_OPTION } from '../general-career/definition.ts';

export const SORT_COLUMNS = [{ key: 'skill', label: 'Skill' }, { key: 'won', label: 'Battles won' }, { key: 'lost', label: 'Battles lost' }, { key: 'unresolved', label: 'Unresolved battles' }, { key: 'winRate', label: 'Win rate' }] as const;
export const DEFAULT_SORT = { column: 'skill', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [
  ...tableSortOptions(SORT_COLUMNS, DEFAULT_SORT),
  { key: 'leaders', section: 'options', label: 'Leaders', kind: 'choice', default: 'all', choices: [
    { value: 'all', label: 'All' }, { value: 'land', label: 'Land' }, { value: 'navy', label: 'Navy' },
  ] }, ...TABLE_OPTIONS, GROW_WITH_PLAYBACK_OPTION, DRILL_OPTION,
];
export const definition = {
  kind: 'generals', version: 1, title: 'Generals leaderboard', icon: 'star',
  description: 'Generals and admirals, their skills, traits and battles.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { tableFamilyOptions, type TableBase } from '../../families/table/settings.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { GROW_WITH_PLAYBACK_OPTION } from '../general-career/definition.ts';

export const DEFAULT_SORT = { column: 'skill', direction: 'descending' } as const;
const GENERALS_TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', overflow: 'scroll', showRank: false, sort: DEFAULT_SORT };

export const OPTIONS: readonly OptionField[] = [
  { key: 'leaders', section: 'options', label: 'Leaders', kind: 'choice', default: 'land', choices: [
    { value: 'land', label: 'Land' }, { value: 'navy', label: 'Navy' }, { value: 'all', label: 'All' },
  ] },
  ...tableFamilyOptions([], GENERALS_TABLE_BASE),
  GROW_WITH_PLAYBACK_OPTION,
  DRILL_OPTION,
];

export const definition = {
  kind: 'generals', version: 1, title: 'Generals leaderboard', icon: 'star',
  description: 'Generals and admirals, their skills, traits and battles.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 }, options: OPTIONS, playsBackDated: true,
} as const satisfies WidgetMeta;

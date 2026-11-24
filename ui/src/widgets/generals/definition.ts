import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';
import { GROW_WITH_PLAYBACK_OPTION } from '../general-career/definition.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'portrait', label: 'Portrait', as: 'plain', offered: ['plain'] },
  { key: 'country', label: 'Country', as: 'flag', offered: OFFERED.country },
  { key: 'commander', label: 'Commander', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'skill', label: 'Skill', as: 'plain', offered: ['plain'] },
  { key: 'attack', label: 'Attack', as: 'plain', offered: ['plain'] },
  { key: 'defense', label: 'Defense', as: 'plain', offered: ['plain'] },
  { key: 'third', label: 'Planning', as: 'plain', offered: ['plain'] },
  { key: 'fourth', label: 'Logistics', as: 'plain', offered: ['plain'] },
  { key: 'traits', label: 'Traits', as: 'plain', offered: ['plain'] },
  { key: 'won', label: 'Battles won', as: 'faint', tone: 'grey', offered: OFFERED.number },
  { key: 'lost', label: 'Battles lost', as: 'faint', tone: 'grey', offered: OFFERED.number },
  { key: 'winRate', label: 'Win rate', as: 'plain', offered: ['plain'] },
  { key: 'fought', label: 'Fought', as: 'faint', tone: 'grey', offered: OFFERED.number, defaultHidden: true },
  { key: 'unresolved', label: 'Unresolved', as: 'faint', tone: 'grey', offered: OFFERED.number, defaultHidden: true },
];
export const DEFAULT_SORT = { column: 'skill', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', showRank: false, sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = [
  { key: 'leaders', section: 'options', label: 'Leaders', kind: 'choice', default: 'all', choices: [
    { value: 'all', label: 'All' }, { value: 'land', label: 'Land' }, { value: 'navy', label: 'Navy' },
  ] },
  ...tableFamilyOptions(COLUMNS, TABLE_BASE),
  GROW_WITH_PLAYBACK_OPTION,
];
export const definition = {
  kind: 'generals', version: 1, title: 'Generals leaderboard', icon: 'star',
  description: 'Generals and admirals, their skills, traits and battles.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'kind', label: 'Division type', as: 'flag', offered: ['flag'] },
  { key: 'country', label: 'Country', as: 'flag', offered: OFFERED.country },
  { key: 'division', label: 'Division', as: 'plain', offered: OFFERED.text },
  { key: 'medals', label: 'Medals', as: 'plain', offered: ['plain'] },
  { key: 'experience', label: 'Experience', as: 'plain', offered: ['plain'] },
  { key: 'enemyKilled', label: 'Enemies killed', as: 'plain', offered: OFFERED.number },
  { key: 'casualtiesTaken', label: 'Casualties taken', as: 'faint', tone: 'grey', offered: OFFERED.number },
];
export const ENEMY_KILLED_NOTE = 'Enemy men this division killed in the battles this recording holds. The game keeps no count of a division’s kills, so each time an enemy division leaves a battle, its losses are shared among the divisions that damaged it, in proportion to the strength damage each dealt. Losses caused by air support and other damage go to no division, and men lost when a division is destroyed are not counted, as in Casualties taken.';
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', sort: { column: 'medals', direction: 'descending' } };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'most-decorated-divisions', version: 1, title: 'Most decorated divisions', icon: 'star', category: 'Army',
  description: 'Every country’s most decorated divisions: medals, experience, enemies killed and casualties taken.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'operative', label: 'Operative', as: 'flagName', offered: ['flagName', 'name'] },
  { key: 'skill', label: 'Skill', as: 'plain', offered: OFFERED.number },
  { key: 'progress', label: 'Next skill', as: 'meterBefore', tone: 'blue', offered: OFFERED.number },
  { key: 'mission', label: 'Mission', as: 'flagName', offered: ['flagName', 'name'] },
  { key: 'state', label: 'State', as: 'dot', tone: 'blue', offered: OFFERED.status },
];
export const DEFAULT_SORT = { column: 'skill', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', overflow: 'scroll', sort: DEFAULT_SORT };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'operatives-roster', version: 1, title: 'Operatives roster', icon: 'shield',
  description: 'A country’s operatives: skill, progress to the next skill, mission and state.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: 1 }, minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

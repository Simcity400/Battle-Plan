import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { FACTION_OPTION } from '../../dashboard/factionOption.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { LINE_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const VIEWS = [{ value: 'time', label: 'Influence over time' }, { value: 'ranking', label: 'Member ranking' }] as const;
export const COLUMNS = [{ key: 'member', label: 'Member' }, { key: 'influence', label: 'Influence share' },
  { key: 'projection', label: 'Power projection share' }, { key: 'contribution', label: 'Contribution' }] as const;
export const DEFAULT_SORT = { column: 'influence', direction: 'descending' } as const;
const ranking = (options: Readonly<Record<string, unknown>>) => options.view === 'ranking';

export const OPTIONS: readonly OptionField[] = [
  FACTION_OPTION,
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'time', choices: VIEWS },
  ...[...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT)].map(field => ({ ...field, visibleWhen: ranking })),
  ...LINE_TIME_OPTIONS.map(field => ({ ...field, visibleWhen: (options: Readonly<Record<string, unknown>>) => !ranking(options) && (field.visibleWhen?.(options as never) ?? true) })),
  DRILL_OPTION,
];

export const definition = {
  kind: 'faction-influence', version: 1, title: 'Faction influence', icon: 'chart-bars', category: 'Diplomacy',
  description: 'Each member’s share of its faction’s influence, over time or ranked.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: 1 }, options: OPTIONS, factionColours: true,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Faction influence ranking'),
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const SORT_COLUMNS = [{ key: 'medals', label: 'Medals' }, { key: 'experience', label: 'Experience' }, { key: 'kills', label: 'Enemy soldiers killed' }] as const;
export const DEFAULT_SORT = { column: 'medals', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [...tableSortOptions(SORT_COLUMNS, DEFAULT_SORT), ...TABLE_OPTIONS, DRILL_OPTION];
export const definition = {
  kind: 'most-decorated-divisions', version: 1, title: 'Most decorated divisions', icon: 'star', category: 'Army',
  description: 'Every country’s most decorated divisions: medals, experience and kills, with each division’s service record.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

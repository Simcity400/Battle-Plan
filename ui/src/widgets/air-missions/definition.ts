import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const DEFAULT_SORT = { column: 'wings', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'theatres', section: 'options', label: 'Air theatres', kind: 'content-set', automatic: 'All recorded theatres', default: '' },
  { key: 'regions', section: 'options', label: 'Regions', kind: 'content-set', automatic: 'All regions with missions', default: '' },
  { key: 'missions', section: 'options', label: 'Missions shown', kind: 'content-set', automatic: 'All missions', default: '' },
  ...TABLE_OPTIONS, ...tableSortOptions([], DEFAULT_SORT, true), DRILL_OPTION,
];
export const definition = {
  kind: 'air-missions', version: 1, title: 'Air missions', icon: 'plane', category: 'Air',
  description: 'Where a country’s air wings fly and on which missions, region by region.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

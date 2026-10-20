import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const COLUMNS = [
  { key: 'country', label: 'Country' }, { key: 'term', label: 'Term' },
  { key: 'days', label: 'Days left' }, { key: 'next', label: 'Next election' },
] as const;
export const DEFAULT_SORT = { column: 'days', direction: 'ascending' } as const;
export const OPTIONS: readonly OptionField[] = [...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT)];
export const definition = {
  kind: 'next-elections', version: 1, title: 'Next elections', category: 'Politics', icon: 'ballot',
  description: 'Countries that hold elections: how far through the term each is, and when it votes next.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const POLICY = { id: 'organisation', label: 'Industrial organisation' };
export const SORT_COLUMNS = [{ key: 'size', label: 'Size' }, { key: 'funds', label: 'Funds' }, { key: 'traits', label: 'Traits unlocked' },
  { key: 'organisation', label: 'Organisation' }] as const;
export const DEFAULT_SORT = { column: 'size', direction: 'descending' } as const;

export const OPTIONS: readonly OptionField[] = [...tableSortOptions(SORT_COLUMNS, DEFAULT_SORT), ...TABLE_OPTIONS];

export const definition = {
  kind: 'industrial-organisations', version: 1, title: 'Industrial organisations', icon: 'factory', category: 'Economy',
  description: 'A country’s industrial organisations: size, funds, traits and policy.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;

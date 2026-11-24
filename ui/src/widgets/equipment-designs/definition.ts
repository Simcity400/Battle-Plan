import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const SHEET = { id: 'design', label: 'Design' };
export const SORT_COLUMNS = [{ key: 'firstSeen', label: 'First seen' }, { key: 'design', label: 'Design' }, { key: 'seenObsolete', label: 'Seen obsolete' }] as const;
export const DEFAULT_SORT = { column: 'firstSeen', direction: 'descending' } as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'included', section: 'options', label: 'Included types', kind: 'content-set', automatic: 'All types', default: '' },
  ...tableSortOptions(SORT_COLUMNS, DEFAULT_SORT), ...TABLE_OPTIONS.filter(field => field.key !== 'cellBars' && field.key !== 'rankMarkers'),
];

export const definition = {
  kind: 'equipment-designs', version: 1, title: 'Equipment designs', icon: 'layers', category: 'Economy',
  description: 'A country’s current and obsolete equipment designs, when each and its obsolete flag were first recorded.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;

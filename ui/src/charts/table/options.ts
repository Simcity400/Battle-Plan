import type { OptionField } from '../../dashboard/options';
import type { Sort } from './logic';

export const TABLE_OPTIONS: readonly OptionField[] = [
  { key: 'cellBars', section: 'options', label: 'Cell bars', kind: 'toggle', default: true },
  { key: 'rankMarkers', section: 'options', label: 'Rank change markers', kind: 'toggle', default: true },
  { key: 'density', section: 'options', label: 'Density', kind: 'choice', default: 'comfortable',
    choices: [{ value: 'comfortable', label: 'Comfortable' }, { value: 'compact', label: 'Compact' }] },
];

/** Persist both parts through the widget's normal option contract. */
export const sortKey = (sort: Sort): string => JSON.stringify([sort.column, sort.direction]);
export function tableSortOf(value: unknown, fallback: Sort): Sort {
  if (typeof value !== 'string') return fallback;
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed) || parsed.length !== 2) return fallback;
    const [column, direction] = parsed;
    return typeof column === 'string' && (direction === 'ascending' || direction === 'descending') ? { column, direction } : fallback;
  } catch { return fallback; }
}
export function tableSortOptions(columns: readonly { key: string; label: string; sortable?: boolean }[], sort: Sort): OptionField[] {
  return [{ key: 'tableSort', section: 'options', label: 'Sort by', kind: 'choice', default: sortKey(sort),
    choices: columns.filter(column => column.sortable !== false).flatMap(column => (['descending', 'ascending'] as const).map(direction => ({
      value: sortKey({ column: column.key, direction }), label: `${column.label} · ${direction === 'descending' ? 'descending' : 'ascending'}`,
    }))) }];
}

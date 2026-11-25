import type { OptionField } from '../../dashboard/options';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { Sort, SortDirection } from './logic';

export const NARROW_TABLE_OPTION: OptionField = { key: 'narrowTable', section: 'options', label: 'When narrow', kind: 'choice', default: 'hide',
  choices: [{ value: 'hide', label: 'Hide less important columns' }, { value: 'scroll', label: 'Scroll sideways' }] };
export type NarrowTable = 'hide' | 'scroll';

export const TABLE_OPTIONS: readonly OptionField[] = [
  { key: 'cellBars', section: 'options', label: 'Cell bars', kind: 'toggle', default: true },
  { key: 'rankMarkers', section: 'options', label: 'Rank change markers', kind: 'toggle', default: true },
  { key: 'density', section: 'options', label: 'Density', kind: 'choice', default: 'comfortable',
    choices: [{ value: 'comfortable', label: 'Comfortable' }, { value: 'compact', label: 'Compact' }] },
  NARROW_TABLE_OPTION,
];

export const sortKey = (sort: Sort): string => JSON.stringify([sort.column, sort.direction]);
export function tableSortOf(value: unknown, fallback: Sort): Sort {
  if (typeof value !== 'string') return fallback;
  try {
    const parsed: unknown = JSON.parse(value.startsWith('[') ? value : contentKey(value));
    if (!Array.isArray(parsed) || parsed.length !== 2) return fallback;
    const [column, direction] = parsed;
    return typeof column === 'string' && (direction === 'ascending' || direction === 'descending') ? { column, direction } : fallback;
  } catch { return fallback; }
}
type SortColumn = { key: string; label: string; sortable?: boolean; direction?: SortDirection };
export const tableSortToken = (sort: Sort): string => setToken(sortKey(sort));
function sortChoices(columns: readonly SortColumn[], token: (sort: Sort) => string): { value: string; label: string }[] {
  return columns.filter(column => column.sortable !== false).flatMap(column => (column.direction ? [column.direction] : ['descending', 'ascending'] as const).map(direction => ({
    value: token({ column: column.key, direction }), label: column.direction ? column.label : `${column.label} · ${direction}`,
  })));
}
export function tableSortChoices(columns: readonly SortColumn[]): { value: string; label: string }[] {
  return sortChoices(columns, tableSortToken);
}
export function tableSortOptions(columns: readonly SortColumn[], sort: Sort, content = false): OptionField[] {
  if (content) return [{ key: 'tableSort', section: 'options', label: 'Sort by', kind: 'content-choice', default: '', automatic: `Default column · ${sort.direction}` }];
  return [{ key: 'tableSort', section: 'options', label: 'Sort by', kind: 'choice', default: sortKey(sort),
    choices: sortChoices(columns, sortKey) }];
}

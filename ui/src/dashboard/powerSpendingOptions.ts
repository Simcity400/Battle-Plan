import { contentSet } from './options.ts';
import type { OptionField, OptionValue } from './options';
import { TABLE_OPTIONS, tableSortOf, tableSortOptions } from '../charts/table/options.ts';
import type { Sort } from '../charts/table/logic';
import { POWER_DECIMALS_OPTION } from './powerDecimals.ts';

export const SPENDING_DEFAULT_SORT: Sort = { column: 'date', direction: 'descending' };
export const SPENDING_SORT_COLUMNS = [
  { key: 'date', label: 'Date' }, { key: 'country', label: 'Country' }, { key: 'item', label: 'Item' },
  { key: 'category', label: 'Category' }, { key: 'amount', label: 'Amount' },
] as const;

export const POWER_SPENDING_OPTIONS: readonly OptionField[] = [
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  { key: 'chips', section: 'options', label: 'Filter chips', kind: 'toggle', default: true },
  { key: 'categories', section: 'options', label: 'Show categories', kind: 'content-set', automatic: 'All categories', default: '' },
  { key: 'filterMode', section: 'options', label: 'Show categories', kind: 'choice', default: 'hide', visibleWhen: () => false,
    choices: [{ value: 'hide', label: 'Show categories' }, { value: 'show', label: 'Show categories' }, { value: 'shown', label: 'Show categories' }] },
  { key: 'totals', section: 'options', label: 'Summary', hint: 'Spent and gained in the shown period.', kind: 'toggle', default: true },
  POWER_DECIMALS_OPTION,
  ...TABLE_OPTIONS.filter(field => field.key !== 'rankMarkers'),
  ...tableSortOptions(SPENDING_SORT_COLUMNS, SPENDING_DEFAULT_SORT),
];

export function spendingSort(saved: Readonly<Record<string, OptionValue>> | undefined): Sort {
  if (typeof saved?.tableSort === 'string') return tableSortOf(saved.tableSort, SPENDING_DEFAULT_SORT);
  if (saved?.order === 'oldest') return { column: 'date', direction: 'ascending' };
  if (saved?.order === 'cost') return { column: 'amount', direction: 'descending' };
  return SPENDING_DEFAULT_SORT;
}

export const NO_SPENDING_CATEGORIES = ':none:';

export function normalizePowerSpendingOptions(saved: Readonly<Record<string, OptionValue>> | undefined,
  known: readonly { value: string }[] | undefined): { categories: string; migrated: boolean } {
  const raw = typeof saved?.categories === 'string' ? saved.categories : '';
  const chosen = contentSet(raw);
  const categories = saved?.filterMode === 'shown' && chosen.length > 1 ? chosen.filter(value => value !== NO_SPENDING_CATEGORIES).join(' ') : raw;
  if (!known || saved?.filterMode === 'shown') return { categories, migrated: false };
  if (!categories && saved?.filterMode !== 'show') return { categories: '', migrated: true };
  if (saved?.filterMode === 'show') return { categories: categories || NO_SPENDING_CATEGORIES, migrated: true };
  const hidden = new Set(contentSet(categories));
  const shown = known.filter(item => item.value !== NO_SPENDING_CATEGORIES && !hidden.has(item.value)).map(item => item.value);
  return { categories: shown.join(' ') || NO_SPENDING_CATEGORIES, migrated: true };
}

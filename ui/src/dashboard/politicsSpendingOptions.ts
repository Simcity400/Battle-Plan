import { contentSet } from './options.ts';
import type { OptionField, OptionValue } from './options';
import { LOG_OPTIONS } from '../charts/log/logOptions.ts';

export const POLITICS_SPENDING_OPTIONS: readonly OptionField[] = [
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  { key: 'categories', section: 'options', label: 'Show categories', kind: 'content-set', automatic: 'All categories', default: '' },
  // Persisted migration marker; legacy choices never appear in settings.
  { key: 'filterMode', section: 'options', label: 'Show categories', kind: 'choice', default: 'hide', visibleWhen: () => false,
    choices: [{ value: 'hide', label: 'Show categories' }, { value: 'show', label: 'Show categories' }, { value: 'shown', label: 'Show categories' }] },
  { key: 'totals', section: 'options', label: 'Running totals', hint: 'Overall total includes all categories.', kind: 'toggle', default: true },
  { key: 'order', section: 'options', label: 'Sort order', kind: 'choice', default: 'newest', choices: [
    { value: 'newest', label: 'Newest first' }, { value: 'oldest', label: 'Oldest first' }, { value: 'cost', label: 'Largest cost first' },
  ] },
  ...LOG_OPTIONS.map(field => field.key === 'months' && field.kind === 'toggle' ? { ...field, default: true, visibleWhen: (options: Readonly<Record<string, OptionValue>>) => options.order !== 'cost' } : field),
];

/** Explicit none preserves legacy filters that hid every known category. */
export const NO_SPENDING_CATEGORIES = ':none:';

/** Resolve legacy Hide/Show selections only after the category catalogue arrives. */
export function normalizePoliticsSpendingOptions(saved: Readonly<Record<string, OptionValue>> | undefined,
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

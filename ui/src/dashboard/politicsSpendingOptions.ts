import type { OptionField } from './options';
import { LOG_OPTIONS } from '../charts/log/logOptions.ts';

export const POLITICS_SPENDING_OPTIONS: readonly OptionField[] = [
  { key: 'artwork', section: 'options', label: 'Game artwork', hint: 'Portraits and icons from this recording’s content', kind: 'toggle', default: true },
  { key: 'categories', section: 'options', label: 'Chosen categories', hint: 'Hidden by default, or shown when Category filter is Show chosen', kind: 'content-set', automatic: 'None chosen', default: '' },
  { key: 'filterMode', section: 'options', label: 'Category filter', hint: 'Hide chosen matches the log’s visible-category chips; Show chosen can isolate categories in recordings with many kinds', kind: 'choice', default: 'hide',
    choices: [{ value: 'hide', label: 'Hide chosen' }, { value: 'show', label: 'Show chosen' }] },
  { key: 'totals', section: 'options', label: 'Running totals', hint: 'All recorded spending by the shown time, including categories hidden by the filter', kind: 'toggle', default: true },
  { key: 'order', section: 'options', label: 'Sort order', kind: 'choice', default: 'newest', choices: [
    { value: 'newest', label: 'Newest first' }, { value: 'oldest', label: 'Oldest first' }, { value: 'cost', label: 'Largest cost first' },
  ] },
  ...LOG_OPTIONS.map(field => field.key === 'months' && field.kind === 'toggle' ? { ...field, default: true, hint: 'Month dividers appear when sorting by date' } : field),
];

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

export const VIEWS = [{ value: 'columns', label: 'Columns' }, { value: 'list', label: 'List' }] as const;
export const SORT_COLUMNS = [
  { key: 'date', label: 'Date' }, { key: 'technology', label: 'Technology' }, { key: 'folder', label: 'Folder' },
  { key: 'slot', label: 'Slot' }, { key: 'took', label: 'Took' },
] as const;
export const DEFAULT_SORT = { column: 'date', direction: 'ascending' } as const;
const inList = (options: Readonly<Record<string, unknown>>) => options.view === 'list';
export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'columns', choices: VIEWS },
  { key: 'folders', section: 'options', label: 'Research folders', kind: 'content-set', automatic: 'All folders', default: '', catalogue: true },
  ...tableSortOptions(SORT_COLUMNS, DEFAULT_SORT).map(field => ({ ...field, visibleWhen: inList })),
  ...TABLE_OPTIONS.filter(field => field.key === 'density' || field.key === 'narrowTable').map(field => ({ ...field, visibleWhen: inList })),
];

export const definition = {
  kind: 'research', version: 2, title: 'Research history', icon: 'flask',
  description: 'The order each research slot worked through, when each technology finished and when slots opened, as columns or a list.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 3, h: 5 }, defaultSize: { w: 7, h: 12 }, options: OPTIONS, viewSwitch: { option: 'view' }, playsBackDated: true,
  migrate: (from, widget) => from < 2 && widget.title === 'Research' ? { ...widget, title: 'Research history' } : widget,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Research list', ['list']),
} as const satisfies WidgetMeta;

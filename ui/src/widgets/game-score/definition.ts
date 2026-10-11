import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';
import { DATE_OVERLAY_OPTION } from '../../charts/race/options.ts';

export const DEFAULT_SORT = { column: 'land', direction: 'descending' } as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'table',
    choices: [{ value: 'table', label: 'Table' }, { value: 'race', label: 'Race' }] },
  { key: 'factions', section: 'options', label: 'Sort by faction', kind: 'toggle', default: false,
    hint: 'Group countries by faction.',
    visibleWhen: options => options.view === 'table' },
  { key: 'multiplierPrecision', section: 'options', label: 'Multiplier decimal places', kind: 'choice', default: 'exact',
    choices: [{ value: 'exact', label: 'Exact' }, ...['0', '1', '2', '3', '4', '5'].map(value => ({ value, label: value }))],
    visibleWhen: options => options.view === 'table' },
  { key: 'topMeasure', section: 'countries', label: 'Top N by', kind: 'content-choice', default: '', automatic: 'Land',
    hint: 'Selects the table’s top countries.',
    visibleWhen: (options, countries) => options.view === 'table' && countries?.kind === 'group' && countries.group.type === 'top' },
  { key: 'split', section: 'options', label: 'Show score parts', kind: 'toggle', default: true,
    hint: 'Split Total into Army, Navy and Air.', visibleWhen: options => options.view === 'race' },
  ...TABLE_OPTIONS.map(field => ({ ...field, visibleWhen: (options: Readonly<Record<string, unknown>>) => options.view === 'table' })),
  ...tableSortOptions([], DEFAULT_SORT, true).map(field => ({ ...field, visibleWhen: (options: Readonly<Record<string, unknown>>) => options.view === 'table' })),
  DATE_OVERLAY_OPTION,
];
export const definition = {
  kind: 'game-score', version: 1, title: 'Game score', icon: 'grid',
  description: 'The game’s current end-screen scores for every country, as a table or a race.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

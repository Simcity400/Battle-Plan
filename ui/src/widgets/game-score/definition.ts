import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS } from '../../charts/table/options.ts';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';
import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS } from '../../charts/race/options.ts';
import { choiceVariants } from '../../dashboard/options.ts';
import { SCORE_LABELS, SCORE_PARTS } from './gameScoreData.ts';

export const DEFAULT_SORT = { column: 'total', direction: 'descending' } as const;
const inTable = (options: Readonly<Record<string, unknown>>) => (options.view ?? 'table') === 'table';
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', showRank: false, sort: DEFAULT_SORT };
const COLUMN_DEFAULTS: readonly TableColumnDefaults[] = [
  { key: 'country', label: SCORE_LABELS.country, as: 'flagName', offered: OFFERED.country, showNamesDefault: true, hideable: false },
  ...SCORE_PARTS.map((part): TableColumnDefaults => ({ key: part, label: SCORE_LABELS[part], as: 'faint', offered: OFFERED.number })),
];
export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'table',
    choices: [{ value: 'table', label: 'Table' }, { value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines over time' }] },
  { key: 'factions', section: 'options', label: 'Sort by faction', kind: 'toggle', default: false, hint: 'Group countries by faction.', visibleWhen: inTable },
  { key: 'topMeasure', section: 'countries', label: 'Rank by', kind: 'choice', default: 'total',
    hint: 'The score a top N, the race and the lines rank by.', choices: SCORE_PARTS.map(part => ({ value: part, label: SCORE_LABELS[part] })) },
  ...TABLE_OPTIONS.filter(field => field.key === 'rankMarkers').map(field => ({ ...field, visibleWhen: inTable })),
  ...tableFamilyOptions(COLUMN_DEFAULTS, TABLE_BASE).map(field => ({ ...field, visibleWhen: inTable })),
  { key: 'split', section: 'options', label: 'Show score parts', kind: 'toggle', default: true,
    hint: 'Split Total into Army, Navy and Air.', visibleWhen: options => options.view === 'race' && (options.topMeasure ?? 'total') === 'total' },
  { ...DATE_OVERLAY_OPTION, visibleWhen: options => options.view === 'race' },
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => options.view === 'lines' },
  ...RACE_TIME_OPTIONS,
  ...RACE_AXIS_OPTIONS,
];
export const definition = {
  kind: 'game-score', version: 3, title: 'Game score', icon: 'grid',
  description: 'The game’s end-screen scores for every country, as a table, a race or lines.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 12 }, options: OPTIONS,
  migrate: (_from, { options: { multiplierPrecision: _precision, split: _split, tableSort: _tableSort, topMeasure, ...options } = {}, ...widget }) => ({ ...widget,
    options: { ...options,
      ...(typeof topMeasure === 'string' && topMeasure ? { topMeasure: topMeasure.replace(/^.*:/, '') } : {}) } }),
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => choice.value === 'race' ? 'Game score race' : `Game score: ${choice.label}`),
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { tableFamilyOptions } from '../../families/table/settings.ts';
import { STOCKPILE_BASE, stockpileColumns } from './stockpileProjection.ts';

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';

const VIEW_OPTION: OptionField = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
  choices: [{ value: 'bars', label: 'Table' }, { value: 'time', label: 'Over time' }] };

export const OPTIONS: readonly OptionField[] = [
  VIEW_OPTION,
  { key: 'layout', section: 'options', label: 'Layout', kind: 'choice', default: 'sum',
    hint: 'Below 470 px wide, Balance and its parts is used',
    choices: [{ value: 'sum', label: 'Sum' }, { value: 'parts', label: 'Balance and its parts' }], visibleWhen: options => !overTime(options) },
  ...tableFamilyOptions(stockpileColumns('sum', null), STOCKPILE_BASE).map(field => ({ ...field, visibleWhen: (options: Readonly<Record<string, OptionValue>>) => !overTime(options) })),
];

export const TIME_OPTIONS: readonly OptionField[] = [
  VIEW_OPTION,
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'stock',
    choices: [{ value: 'stock', label: 'In stock' }, { value: 'share', label: 'Surplus or shortage, share of need' }] },
  { key: 'chosen', section: 'options', label: 'Types shown', kind: 'content-set', automatic: 'Main types at the date', default: '' },
  { key: 'endLabels', section: 'options', label: 'Names at the line ends', kind: 'toggle', default: true },
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'equipment-stockpile', version: 1, title: 'Equipment stockpile', icon: 'grid',
  description: 'What a country holds of each equipment type and model, where the game says it is short, and what it makes a day.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 13 },
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
  variants: choiceVariants(VIEW_OPTION, () => 'Stockpile over time'),
} as const satisfies WidgetMeta;

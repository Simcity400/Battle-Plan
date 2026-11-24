import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { NARROW_TABLE_OPTION } from '../../charts/table/options.ts';

export const BREAKDOWN = { id: 'stockpile', label: 'Stockpile breakdown' };

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';

const VIEW_OPTION: OptionField = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
  choices: [{ value: 'bars', label: 'Table' }, { value: 'time', label: 'Over time' }] };

export const OPTIONS: readonly OptionField[] = [
  { key: 'stockpileCountry', section: 'options', label: 'Selected country', kind: 'content-choice', automatic: 'Selected country', default: '', visibleWhen: () => false },
  breakdownOption([BREAKDOWN], 'Equipment stockpile'), DRILL_OPTION,
  { key: 'partGroup', section: 'options', label: 'Breakdown equipment', kind: 'content-choice', default: '',
    automatic: 'Choose an equipment row', visibleWhen: options => options.show === BREAKDOWN.id },
  VIEW_OPTION,
  { key: 'count', section: 'options', label: 'Count', kind: 'choice', default: 'units',
    hint: 'IC shows output as the IC its lines put in a day. Stock and surplus stay in units: the game gives stock no IC value.',
    choices: [{ value: 'units', label: 'Units' }, { value: 'ic', label: 'IC' }], visibleWhen: options => !overTime(options) },
  { ...NARROW_TABLE_OPTION, visibleWhen: options => !overTime(options) },
  { key: 'included', section: 'options', label: 'Included types', kind: 'content-set',
    automatic: 'All types', default: '' },
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
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS, viewSwitch: { option: 'count' },
  variants: choiceVariants(VIEW_OPTION, () => 'Stockpile over time'),
} as const satisfies WidgetMeta;

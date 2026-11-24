import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION, choiceVariants } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { tableFamilyOptions } from '../../families/table/settings.ts';
import { STOCKPILE_BASE, stockpileColumns } from './stockpileProjection.ts';

export const BREAKDOWN = { id: 'stockpile', label: 'Stockpile breakdown' };

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';

export const OPTIONS: readonly OptionField[] = [
  { key: 'stockpileCountry', section: 'options', label: 'Selected country', kind: 'content-choice', automatic: 'Selected country', default: '', visibleWhen: () => false },
  breakdownOption([BREAKDOWN], 'Equipment stockpile'), DRILL_OPTION,
  { key: 'partGroup', section: 'options', label: 'Breakdown equipment', kind: 'content-choice', default: '',
    automatic: 'Choose an equipment row', visibleWhen: options => options.show === BREAKDOWN.id },
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
    choices: [{ value: 'bars', label: 'Table' }, { value: 'time', label: 'Over time' }] },
  { key: 'layout', section: 'options', label: 'Layout', kind: 'choice', default: 'sum',
    hint: 'Below 470 px wide, Balance and its parts is used',
    choices: [{ value: 'sum', label: 'Sum' }, { value: 'parts', label: 'Balance and its parts' }], visibleWhen: options => !overTime(options) },
  ...tableFamilyOptions(stockpileColumns('sum', null), STOCKPILE_BASE).map(field => ({ ...field, visibleWhen: (options: Readonly<Record<string, OptionValue>>) => !overTime(options) })),
  { key: 'included', section: 'options', label: 'Included types', kind: 'content-set',
    automatic: 'All types', default: '', visibleWhen: overTime },
  { key: 'chosen', section: 'options', label: 'Types shown over time', kind: 'content-set',
    automatic: 'Main types', default: '', visibleWhen: overTime },
  { key: 'endLabels', section: 'options', label: 'Names at the line ends', kind: 'toggle', default: true, visibleWhen: overTime },
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in', visibleWhen: overTime,
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { ...AFTER_PLAYHEAD_OPTION, visibleWhen: options => overTime(options) && options.playback === 'cursor' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => overTime(options) && options.playback === 'window' },
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false, visibleWhen: overTime },
];

export const definition = {
  kind: 'equipment-stockpile', version: 1, title: 'Equipment stockpile', icon: 'grid',
  description: 'What a country holds of each equipment type and model, where the game says it is short, and what it makes a day.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 13 },
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Stockpile over time'),
} as const satisfies WidgetMeta;

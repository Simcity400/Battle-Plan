import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { GROUPINGS, MEASURES } from './producedData.ts';

/** Most countries one widget ranks or draws. */
export const MAX_COUNTRIES = 30;
/** Most types the included set can pick (a content set's limit). */
export const MAX_INCLUDED = 16;

const view = (options: Readonly<Record<string, OptionValue>>) => options.view ?? 'race';

export const OPTIONS: readonly OptionField[] = [
  raceViewOption('equipment'),
  { key: 'measure', section: 'options', label: 'Measure', hint: 'Per week and per month count what was delivered in the week or month before the moment shown',
    kind: 'choice', default: 'total', choices: MEASURES.map(item => ({ value: item.value, label: item.label })) },
  { key: 'grouping', section: 'options', label: 'Group by', kind: 'choice', default: 'type',
    choices: GROUPINGS.map(item => ({ value: item.value, label: item.label })) },
  { key: 'included', section: 'options', label: 'Included types', hint: 'From the campaign’s own content', kind: 'content-set',
    automatic: 'All types', max: MAX_INCLUDED, default: '' },
  { key: 'refits', section: 'options', label: 'Count refitted ships', hint: 'A refit rebuilds a ship the country already had as another design',
    kind: 'toggle', default: false },
  { key: 'split', section: 'options', label: 'Split bars by group', hint: 'One segment per type, variant or arm instead of one total', kind: 'toggle', default: false,
    visibleWhen: options => view(options) === 'race' },
  DATE_OVERLAY_OPTION,
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => view(options) === 'lines' },
  ...RACE_TIME_OPTIONS,
  ...RACE_AXIS_OPTIONS,
];

export const definition = {
  kind: 'equipment-produced', version: 1, title: 'Equipment produced', icon: 'layers',
  description: 'Equipment each country has produced: a race re-ranked every hour, lines over time or one country by type.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: MAX_COUNTRIES, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;

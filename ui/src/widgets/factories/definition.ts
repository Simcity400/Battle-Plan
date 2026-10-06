import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { COUNT_MODES } from './factoriesData.ts';

const view = (options: Readonly<Record<string, OptionValue>>) => options.view ?? 'race';

export const OPTIONS: readonly OptionField[] = [
  raceViewOption('type'),
  { key: 'count', section: 'options', label: 'Count', hint: 'Free after withheld: what is left once damaged, consumer goods, trade, projects, licences, contracts and factories given away are taken out',
    kind: 'choice', default: 'free',
    choices: COUNT_MODES.map(mode => ({ value: mode.value, label: mode.label })) },
  { key: 'types', section: 'options', label: 'Factory types', hint: 'From the campaign’s own buildings', kind: 'content-set',
    automatic: 'All types', default: '' },
  { key: 'split', section: 'options', label: 'Split bars by type', hint: 'One segment per factory type instead of one total', kind: 'toggle', default: false,
    visibleWhen: options => view(options) === 'race' },
  DATE_OVERLAY_OPTION,
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => view(options) === 'lines' },
  ...RACE_TIME_OPTIONS,
  ...RACE_AXIS_OPTIONS,
];

export const definition = {
  kind: 'factories', version: 1, title: 'Factories', icon: 'factory',
  description: 'Factory counts per country: a race re-ranked every hour, lines over time or one country by type.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;

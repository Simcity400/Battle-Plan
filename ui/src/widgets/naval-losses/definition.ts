import type { WidgetMeta } from '../../dashboard/definitions';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import type { OptionField } from '../../dashboard/options';
import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, TOP_COUNTRIES } from '../../charts/race/options.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'race',
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines over time' }] },
  { key: 'kind', section: 'options', label: 'Count', kind: 'choice', default: 'both',
    choices: [{ value: 'ships', label: 'Ships' }, { value: 'convoys', label: 'Convoys' }, { value: 'both', label: 'Ships and convoys' }] },
  { key: 'attribution', section: 'options', label: 'Attribution', kind: 'choice', default: 'lost',
    choices: [{ value: 'lost', label: 'Lost' }, { value: 'sunkBy', label: 'Sunk by' }] },
  { key: 'period', section: 'options', label: 'Period', kind: 'choice', default: 'running',
    choices: [{ value: 'running', label: 'Running total' }, { value: 'month', label: 'Per month' }] },
  DRILL_OPTION, breakdownOption([{ id: 'parts', label: 'Ships and convoys breakdown' }], 'Naval losses'),
  { key: 'drillOriginOffset', section: 'options', label: 'Country selection offset', kind: 'number',
    default: null, min: 0, max: 4294967295, step: 1, visibleWhen: () => false },
  { key: 'drillOriginTag', section: 'options', label: 'Country selection tag', kind: 'content-choice',
    automatic: 'Country selection', default: '', visibleWhen: () => false },
  DATE_OVERLAY_OPTION,
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => options.view === 'lines' },
  ...RACE_TIME_OPTIONS, ...RACE_AXIS_OPTIONS,
];

export const definition = {
  kind: 'naval-losses', version: 1, title: 'Naval losses', icon: 'shield',
  description: 'Ships and convoys lost or sunk by each country, as a race or lines over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, options: OPTIONS,
} as const satisfies WidgetMeta;

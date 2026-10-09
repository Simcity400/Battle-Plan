import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  breakdownOption([{ id: 'breakdown', label: 'Army breakdown' }], 'Army size'),
  { ...raceViewOption('unit group (battalions)'), visibleWhen: options => options.show !== 'breakdown' },
  DRILL_OPTION,
  { key: 'drillOriginHour', section: 'options', label: 'Recorded country selection hour', kind: 'number',
    default: null, min: -2147483648, max: 2147483647, step: 1, visibleWhen: () => false },
  { key: 'measure', section: 'options', label: 'Measure', kind: 'content-choice', default: '',
    automatic: 'Default for this recording', titleChoice: false, visibleWhen: options => options.show === 'breakdown' || options.view !== 'mix',
    hint: 'Manpower in the field when recorded; otherwise number of divisions' },
  { key: 'countFor', section: 'options', label: 'Expeditionary forces count for', kind: 'choice', default: 'owner',
    choices: [{ value: 'owner', label: 'Owner' }, { value: 'host', label: 'Host' }] },
  { key: 'byType', section: 'options', label: 'Battalions by unit group', kind: 'toggle', default: false,
    hint: 'Applies to battalion counts; divisions and HP stay unsplit',
    visibleWhen: options => options.show !== 'breakdown' && (options.view ?? 'race') === 'race' && options.measure === 'battalions' },
  { ...DATE_OVERLAY_OPTION, visibleWhen: options => options.show !== 'breakdown' && (options.view ?? 'race') === 'race' },
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => options.show !== 'breakdown' && options.view === 'lines' },
  ...RACE_TIME_OPTIONS.map(field => ({ ...field, visibleWhen: options => field.visibleWhen?.(options.show === 'breakdown' ? { ...options, view: 'mix' } : options) ?? true } satisfies OptionField)),
  ...RACE_AXIS_OPTIONS.map(field => ({ ...field, visibleWhen: options => field.visibleWhen?.(options.show === 'breakdown' ? { ...options, view: 'mix' } : options) ?? true } satisfies OptionField)),
];

export const definition = {
  kind: 'army-size', version: 1, title: 'Army size', icon: 'shield',
  description: 'A land army race, country lines or battalions by unit group.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES, options: OPTIONS,
} as const satisfies WidgetMeta;

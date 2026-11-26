import { RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'race', choices: [
    { value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines' },
    { value: 'mix', label: 'Battalion mix' }, { value: 'breakdown', label: 'Army breakdown' },
  ] },
  { key: 'measure', section: 'options', label: 'Measure', kind: 'content-choice', default: '',
    automatic: 'Default for this recording', titleChoice: false, visibleWhen: options => options.view !== 'mix',
    hint: 'Manpower in the field when recorded; otherwise number of divisions' },
  { key: 'countFor', section: 'options', label: 'Expeditionary forces count for', kind: 'choice', default: 'owner',
    choices: [{ value: 'owner', label: 'Owner' }, { value: 'host', label: 'Host' }] },
  { key: 'byType', section: 'options', label: 'Battalions by unit group', kind: 'toggle', default: false,
    visibleWhen: options => (options.view ?? 'race') === 'race' && options.measure === 'battalions' },
  ...RACE_TIME_OPTIONS.map(field => ({ ...field, visibleWhen: options => field.visibleWhen?.(options.view === 'breakdown' ? { ...options, view: 'mix' } : options) ?? true } satisfies OptionField)),
  ...RACE_AXIS_OPTIONS.map(field => ({ ...field, visibleWhen: options => field.visibleWhen?.(options.view === 'breakdown' ? { ...options, view: 'mix' } : options) ?? true } satisfies OptionField)),
];

export const definition = {
  kind: 'army-size', version: 2, title: 'Army size', icon: 'shield',
  description: 'A land army race, country lines or battalions by unit group.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES, options: OPTIONS,
  variants: raceViewVariants('Army size', OPTIONS, { mix: 'Army composition', breakdown: 'Army breakdown' }),
} as const satisfies WidgetMeta;

import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { REACHED_COUNTRY_OPTION } from '../../dashboard/reachedCountry.ts';

export const MEASURES = [{ value: 'manpower', label: 'Manpower' }, { value: 'equipment', label: 'Equipment' }] as const;
export const LEDGER_CHOICES = [{ value: 'combat', label: 'Lost in combat' }, { value: 'attrition', label: 'Lost to attrition' },
  { value: 'recovered', label: 'Recovered' }, { value: 'enemy', label: 'Enemy losses dealt' }] as const;
export const OPTIONS: readonly OptionField[] = [
  REACHED_COUNTRY_OPTION,
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'manpower', choices: [...MEASURES] },
  { key: 'manpowerStack', section: 'options', label: 'Stack', kind: 'choice', default: 'channel', visibleWhen: options => options.measure !== 'equipment',
    choices: [{ value: 'channel', label: 'By source (Ground, Air, Navy)' }, { value: 'armyGroup', label: 'By army group (current)' }] },
  { key: 'ledger', section: 'options', label: 'Ledger', kind: 'choice', default: 'combat', visibleWhen: options => options.measure === 'equipment', choices: [...LEDGER_CHOICES] },
  { key: 'equipmentStack', section: 'options', label: 'Stack', kind: 'choice', default: 'type', visibleWhen: options => options.measure === 'equipment',
    choices: [{ value: 'type', label: 'By equipment type' }, { value: 'armyGroup', label: 'By army group (current)' }] },
  { key: 'timeAxis', section: 'axes', label: 'Time axis', kind: 'choice', default: 'fold',
    choices: [{ value: 'fold', label: 'Fold unrecorded months' }, { value: 'true', label: 'True time' }] },
];
export const definition = {
  kind: 'army-losses-by-month', version: 2, title: 'Army losses by month', icon: 'chart-line', category: 'Army',
  description: 'A country\u2019s army losses per month from the game\u2019s combat log, by source, army group or equipment type.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: 1 }, minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 8 }, options: OPTIONS, viewSwitch: { option: 'measure' },
} as const satisfies WidgetMeta;

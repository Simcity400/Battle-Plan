import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const SHOWS = [{ value: 'conditions', label: 'Conditions' }, { value: 'temperature', label: 'Temperature' }, { value: 'modifiers', label: 'Modifiers' }] as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'conditions', choices: SHOWS },
  { key: 'modifier', section: 'options', label: 'Modifier', kind: 'content-choice', automatic: 'First recorded modifier', default: '',
    visibleWhen: o => o.show === 'modifiers' },
  { key: 'theatres', section: 'options', label: 'Air theatres', kind: 'content-set', automatic: 'All recorded theatres', default: '' },
  { key: 'regions', section: 'options', label: 'Regions', kind: 'content-set', automatic: 'All regions', default: '' },
  DRILL_OPTION,
];
export const definition = {
  kind: 'weather-by-region', version: 1, title: 'Weather by region', icon: 'sun', category: 'Air',
  description: 'Recorded weather by strategic region, day by day: conditions, temperature and weather modifiers.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, countrySelection: false, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 8 }, options: OPTIONS, viewSwitch: { option: 'show' },
  variants: choiceVariants(OPTIONS.find(field => field.key === 'show'), choice => `Weather by region: ${choice.label}`),
} as const satisfies WidgetMeta;

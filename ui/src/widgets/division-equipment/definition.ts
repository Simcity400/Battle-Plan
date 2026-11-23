import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { REACHED_COUNTRY_OPTION } from '../../dashboard/reachedCountry.ts';
import { choiceVariants } from '../../dashboard/options.ts';
import { TABLE_OPTIONS } from '../../charts/table/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { COUNT_FOR_OPTION } from '../division-template/definition.ts';

export const VIEWS = [{ value: 'equipment', label: 'By equipment' }, { value: 'template', label: 'By template' }] as const;
export const VIEW_OPTION: OptionField = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'equipment', choices: [...VIEWS] };
export const OPTIONS: readonly OptionField[] = [REACHED_COUNTRY_OPTION, VIEW_OPTION, COUNT_FOR_OPTION, ...TABLE_OPTIONS.filter(option => option.key !== 'rankMarkers'), DRILL_OPTION];
export const definition = {
  kind: 'division-equipment', version: 1, title: 'Division equipment', icon: 'helmet', category: 'Army',
  description: 'What a country’s divisions hold, need and are missing, with manpower and strength, from the army down to one division.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: 1 }, minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, options: OPTIONS, viewSwitch: { option: 'view' },
  variants: choiceVariants(VIEW_OPTION, choice => `Division equipment ${choice.label.toLowerCase()}`),
} as const satisfies WidgetMeta;

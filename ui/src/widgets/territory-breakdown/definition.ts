import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { MEASURES } from '../../dashboard/territoryMeasure.ts';
import { BREAKDOWN_OPTIONS } from '../../families/breakdown/options.ts';

export const OPTIONS: readonly OptionField[] = [
  ...BREAKDOWN_OPTIONS,
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'states', choices: MEASURES },
  { key: 'rule', section: 'options', label: 'Territory rule', kind: 'choice', default: 'controlled',
    choices: [{ value: 'controlled', label: 'Controlled' }, { value: 'owned', label: 'Owned' }] },
];

export const definition = {
  kind: 'territory-breakdown', version: 1, title: 'Territory breakdown', category: 'Diplomacy', icon: 'sunburst',
  description: 'Territory by country and strategic region.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 }, options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;

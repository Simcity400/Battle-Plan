import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';
import { MEASURES } from '../../dashboard/territoryMeasure.ts';

export const LEVELS = ['faction', 'country', 'region'] as const;
export const OPTIONS: readonly OptionField[] = [
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'states', choices: MEASURES },
  { key: 'rule', section: 'options', label: 'Territory rule', kind: 'choice', default: 'controlled',
    choices: [{ value: 'controlled', label: 'Controlled' }, { value: 'owned', label: 'Owned' }] },
  ...SUNBURST_OPTIONS, levelOption('faction', 'Faction ring'), levelOption('country', 'Country ring'), levelOption('region', 'Strategic region ring'),
];
export const definition = {
  kind: 'territory-breakdown', version: 1, title: 'Territory breakdown', category: 'Diplomacy', icon: 'sunburst',
  description: 'Territory grouped by faction, country and strategic region.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  factionColours: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;

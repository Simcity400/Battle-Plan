import { RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, raceViewVariants } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';
import { MEASURES as TERRITORY_MEASURES } from '../../dashboard/territoryMeasure.ts';
import { COUNT_MODES, MEASURES } from './factionStrengthData.ts';
import { factionMemberChoice, factionMemberOptions } from '../../charts/race/factionMembersOptions.ts';
import { breakdownVariants } from '../../charts/drill/drillState.ts';

export const OPTIONS: readonly OptionField[] = [
  raceViewOption(),
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'factories', choices: MEASURES },
  { key: 'count', section: 'options', label: 'Factory count', kind: 'choice', default: 'free', choices: COUNT_MODES,
    visibleWhen: options => options.measure !== 'territory' },
  { key: 'territoryMeasure', section: 'options', label: 'Territory measure', kind: 'choice', default: 'states', choices: TERRITORY_MEASURES,
    visibleWhen: options => options.measure === 'territory' },
  { key: 'rule', section: 'options', label: 'Territory rule', kind: 'choice', default: 'controlled',
    choices: [{ value: 'controlled', label: 'Controlled' }, { value: 'owned', label: 'Owned' }], visibleWhen: options => options.measure === 'territory' },
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => options.view === 'lines' },
  ...RACE_TIME_OPTIONS,
  ...RACE_AXIS_OPTIONS,
  ...factionMemberOptions.map(option => ({ ...option, visibleWhen: (_options: Readonly<Record<string, unknown>>, countries?: Parameters<typeof factionMemberChoice>[0]) => factionMemberChoice(countries) })),
];

export const definition = {
  kind: 'faction-strength', version: 1, title: 'Faction strength', category: 'Diplomacy', icon: 'shield',
  description: 'Faction factories and territory over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, aggregate: true, factions: true },
  factionColours: true,
  defaultCountries: { kind: 'factions', count: Number.MAX_SAFE_INTEGER, membership: 'hour' },
  options: OPTIONS,
  variants: [...raceViewVariants('Faction strength', OPTIONS), ...breakdownVariants('Faction strength', OPTIONS),
    ...choiceVariants(OPTIONS.find(field => field.key === 'measure'), () => 'Faction territory', ['territory'])],
} as const satisfies WidgetMeta;

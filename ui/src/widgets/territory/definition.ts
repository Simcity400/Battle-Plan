import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { factionMemberChoice, factionMemberOptions } from '../../charts/race/factionPartsOptions.ts';

const linesOnly = (option: OptionField): OptionField => ({ ...option,
  visibleWhen: options => options.view === 'lines' && (option.visibleWhen?.(options) ?? true) });
export const OPTIONS: readonly OptionField[] = [
  raceViewOption(undefined, 'race', [{ value: 'share', label: 'Share' }]),
  { key: 'measure', section: 'options', label: 'Measure', kind: 'content-choice', automatic: 'States', default: '' },
  { key: 'rule', section: 'options', label: 'Territory rule', kind: 'choice', default: 'controlled',
    choices: [{ value: 'controlled', label: 'Controlled' }, { value: 'owned', label: 'Owned' }] },
  DATE_OVERLAY_OPTION,
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => options.view === 'lines' },
  ...RACE_TIME_OPTIONS.map(linesOnly),
  ...RACE_AXIS_OPTIONS.map(linesOnly),
  ...factionMemberOptions.map(option => ({ ...option, visibleWhen: (options, countries) => factionMemberChoice(countries)
    && (option.visibleWhen?.(options, countries) ?? true) } satisfies OptionField)),
];

export const definition = {
  kind: 'territory', version: 1, title: 'Territory', icon: 'map',
  description: 'How much of the world each country holds: a race, lines over time or a world share.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, aggregate: true, factions: true }, defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;

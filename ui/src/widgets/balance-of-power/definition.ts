import type { WidgetMeta } from '../../dashboard/definitions';
import type { CountrySelection } from '../../dashboard/model';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';
import type { Breakdown } from '../../charts/drill/drillState';
import { TABLE_OPTIONS, tableSortOptions } from '../../charts/table/options.ts';

/** The breakdown a click on the line opens, and that Pin as widget shows as the widget itself. */
export const WEEKLY_BREAKDOWN: Breakdown = { id: 'weekly', label: 'Weekly change' };

/** One country (the dashboard's, or one picked) shows its balance over time; any other
 * selection lists every active balance. */
export const singleCountry = (countries: CountrySelection | undefined) =>
  !countries || countries.kind === 'selected' || countries.kind === 'tags' && countries.tags.length === 1;

export const COLUMNS = [
  { key: 'balance', label: 'Balance' }, { key: 'members', label: 'Members' }, { key: 'range', label: 'Range' },
  { key: 'value', label: 'Value' }, { key: 'trending', label: 'Leaning toward' },
] as const;
/** Strongest lean first. */
export const DEFAULT_SORT = { column: 'value', direction: 'descending' } as const;

const oneCountry = (_options: Readonly<Record<string, OptionValue>>, countries?: CountrySelection) => singleCountry(countries);
const list = (_options: Readonly<Record<string, OptionValue>>, countries?: CountrySelection) => !singleCountry(countries);
const chart = (options: Readonly<Record<string, OptionValue>>, countries?: CountrySelection) => options.show === 'main' && singleCountry(countries);

export const OPTIONS: readonly OptionField[] = [
  { ...breakdownOption([WEEKLY_BREAKDOWN], 'Balance'), visibleWhen: oneCountry },
  { key: 'balance', section: 'options', label: 'Balance', kind: 'content-choice', automatic: 'The one the game shows', default: '', titleChoice: false,
    visibleWhen: oneCountry },
  ...[...TABLE_OPTIONS, ...tableSortOptions(COLUMNS, DEFAULT_SORT)].map(field => ({ ...field, visibleWhen: list })),
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }],
    visibleWhen: chart },
  { ...AFTER_PLAYHEAD_OPTION, visibleWhen: chart },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: (options, countries) => chart(options, countries) && options.playback === 'window' },
  { ...DRILL_OPTION, visibleWhen: chart },
];

export const definition = {
  kind: 'balance-of-power', version: 1, title: 'Balance of power', category: 'Politics', icon: 'sliders',
  description: 'Each internal power struggle over time: where it stands, its ranges and what moves it.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'selected' },
  options: OPTIONS,
  variants: breakdownVariants('Balance of power', OPTIONS),
} as const satisfies WidgetMeta;

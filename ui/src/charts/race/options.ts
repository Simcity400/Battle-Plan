/** Settings every race widget shares (Factories, Equipment produced, Territory,
 * Army size…): the view (race, lines over time, one country's mix; spec: the same
 * data in a similar chart is a setting, not another widget), the corner date, the
 * line chart's playback, and the default top 10 of all countries. */
import { DEFAULT_TOP } from '../../dashboard/groups.ts';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import type { CountrySelection } from '../../dashboard/model';

export type RaceView = 'race' | 'lines' | 'mix';

/** A race widget's default countries: the top 10 of all countries, re-ranked at
 * every hour. */
export const TOP_COUNTRIES: CountrySelection = { kind: 'group', group: { type: 'top', count: DEFAULT_TOP }, membership: 'hour', aggregate: false };

/** The country one country's mix shows: the first chosen, the dashboard's, or for a
 * group the member ranked first at the moment shown; never one outside the group
 * then, so null when none of it was recorded. */
export function mixCountry(countries: CountrySelection, fallback: string | null, ranked: readonly { tag: string }[]): string | null {
  if (countries.kind === 'tags') return countries.tags[0] ?? null;
  if (countries.kind === 'selected') return fallback;
  return ranked[0]?.tag ?? null;
}

/** The view: `mix` names what one country's mix is split by (such as "type"). */
export function raceViewOption(mixLabel?: string, defaultView: RaceView = 'race',
  extraViews: readonly { value: string; label: string }[] = []): OptionField {
  return { key: 'view', section: 'options', label: 'View', kind: 'choice', default: defaultView,
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines over time' },
      ...(mixLabel === undefined ? [] : [{ value: 'mix', label: `One country by ${mixLabel}` }]), ...extraViews] };
}

const inRace = (options: Readonly<Record<string, unknown>>) => (options.view ?? 'race') === 'race';
const overTime = (options: Readonly<Record<string, unknown>>) => !inRace(options);

/** The moment shown as a large date in the race's corner; off by default, the same
 * rule for every race widget. */
export const DATE_OVERLAY_OPTION: OptionField = { key: 'dateOverlay', section: 'options', label: 'Date in the corner',
  hint: 'The moment shown, large and faint behind the bars', kind: 'toggle', default: false, visibleWhen: inRace };

/** The time charts' playback settings, shown for the lines and mix views. */
export const RACE_TIME_OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in', visibleWhen: overTime,
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { ...AFTER_PLAYHEAD_OPTION, visibleWhen: options => overTime(options) && options.playback === 'cursor' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => overTime(options) && options.playback === 'window' },
];

/** The time charts' value axis. */
export const RACE_AXIS_OPTIONS: readonly OptionField[] = [
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty starts at zero', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false, visibleWhen: options => options.view === 'lines' },
];

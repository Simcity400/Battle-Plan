import { DEFAULT_TOP } from '../../dashboard/groups.ts';
import { AFTER_PLAYHEAD_OPTION, choiceVariants } from '../../dashboard/options.ts';
import type { WidgetPreset } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import type { CountrySelection } from '../../dashboard/model';

export type RaceView = 'race' | 'lines' | 'mix';

export const TOP_COUNTRIES: CountrySelection = { kind: 'group', group: { type: 'top', count: DEFAULT_TOP }, membership: 'hour', aggregate: false };

export function raceViewOption(mixLabel?: string, defaultView: RaceView = 'race',
  extraViews: readonly { value: string; label: string }[] = []): OptionField {
  return { key: 'view', section: 'options', label: 'View', kind: 'choice', default: defaultView,
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines' },
      ...(mixLabel === undefined ? [] : [{ value: 'mix', label: `By ${mixLabel}` }]), ...extraViews] };
}

const VIEW_TITLES: Readonly<Record<string, (title: string) => string>> = {
  race: title => `${title} race`, lines: title => `${title} over time`, share: title => `${title} share` };

export function raceViewVariants(title: string, fields: readonly OptionField[], names: Readonly<Record<string, string>> = {}): WidgetPreset[] {
  return choiceVariants(fields.find(field => field.key === 'view'), choice =>
    names[choice.value] ?? VIEW_TITLES[choice.value]?.(title) ?? `${title}: ${choice.label}`);
}

const inRace = (options: Readonly<Record<string, unknown>>) => (options.view ?? 'race') === 'race';
const overTime = (options: Readonly<Record<string, unknown>>) => !inRace(options);

export const DATE_OVERLAY_OPTION: OptionField = { key: 'dateOverlay', section: 'options', label: 'Date in the corner',
  kind: 'toggle', default: false, visibleWhen: inRace };

export const RACE_TIME_OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in', visibleWhen: overTime,
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { ...AFTER_PLAYHEAD_OPTION, visibleWhen: options => overTime(options) && options.playback === 'cursor' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => overTime(options) && options.playback === 'window' },
];

export const RACE_AXIS_OPTIONS: readonly OptionField[] = [
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty starts at zero', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false, visibleWhen: options => options.view === 'lines' },
];

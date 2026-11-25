import { choiceVariants } from '../../dashboard/options.ts';
import type { WidgetPreset } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { at, FOLLOW, regionFollows, SPEC, type Region, type RegionAction } from '../../dashboard/regionTime.ts';

export interface Breakdown { id: string; label: string }
export interface DrillHolder { tag: string; origin: number }
export interface DrillState { breakdown: string | null; spec: boolean; follow: boolean; hour: number | null; country?: DrillHolder | null }
export const CLOSED: DrillState = { breakdown: null, spec: true, follow: false, hour: null };
export const savedDrill = (breakdown: string | null): DrillState => breakdown === null ? CLOSED : { breakdown, spec: true, follow: false, hour: null };

export const FOLLOW_NEAR_HOURS = 45 * 24;

export type DrillAction =
  | { type: 'open'; breakdown: string; hour: number; follow: boolean; country?: DrillHolder | null }
  | { type: 'attach'; breakdown: string }
  | { type: 'pick'; hour: number }
  | { type: 'follow' }
  | { type: 'close' };

function uncountried<S extends DrillState>(state: S): S {
  const { country, ...rest } = state;
  return country === undefined ? state : rest as S;
}

export function reduceDrill<S extends DrillState>(state: S, action: DrillAction): S {
  switch (action.type) {
    case 'open': return { ...uncountried(state), breakdown: action.breakdown, spec: false, follow: action.follow, hour: action.follow ? null : Math.floor(action.hour),
      ...action.country ? { country: { tag: action.country.tag, origin: Math.floor(action.country.origin) } } : {} };
    case 'attach': return { ...state, breakdown: action.breakdown, spec: true, follow: false, hour: null };
    case 'pick': return state.breakdown === null ? state : { ...state, spec: false, follow: false, hour: Math.floor(action.hour) };
    case 'follow': return state.breakdown === null ? state : { ...state, spec: false, follow: true, hour: null };
    case 'close': return { ...uncountried(state), ...CLOSED };
  }
}

export function drillAction(drill: DrillState): RegionAction {
  return drill.spec ? SPEC : drill.follow ? FOLLOW : at(drill.hour ?? Number.NaN);
}

export function drillFollows(parent: Region, drill: DrillState): boolean {
  return drill.spec ? drill.breakdown !== null && regionFollows(parent) : drill.follow;
}

export function opensFollowing(parent: Region, hour: number, playing: boolean, playhead: number): boolean {
  return regionFollows(parent) && (playing || Math.abs(hour - playhead) <= FOLLOW_NEAR_HOURS);
}

export const DRILL_OPTION: OptionField = {
  key: 'drill', section: 'interaction', label: 'Breakdown opens',
  kind: 'choice', default: 'inline', choices: [{ value: 'inline', label: 'In the widget' }, { value: 'popout', label: 'Beside it' }],
};

export function breakdownOption(breakdowns: readonly Breakdown[], mainLabel: string): OptionField {
  return { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'main',
    choices: [{ value: 'main', label: mainLabel }, ...breakdowns.map(item => ({ value: item.id, label: item.label }))] };
}

export const breakdownTitle = (title: string, label: string) => `${title}: ${label}`.slice(0, 160);

export function breakdownVariants(title: string, fields: readonly OptionField[]): WidgetPreset[] {
  return choiceVariants(fields.find(field => field.key === 'show'), choice => breakdownTitle(title, choice.label));
}

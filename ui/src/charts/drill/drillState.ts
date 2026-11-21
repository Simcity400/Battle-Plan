import { createWidget } from '../../dashboard/model.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { choiceVariants } from '../../dashboard/options.ts';
import type { WidgetPreset } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';


export interface Breakdown { id: string; label: string }
export interface DrillState { breakdown: string | null; follow: boolean; hour: number | null }
export const CLOSED: DrillState = { breakdown: null, follow: true, hour: null };

export const FOLLOW_NEAR_HOURS = 45 * 24;

export type DrillAction =
  | { type: 'open'; breakdown: string; hour: number; playing: boolean; playhead: number | null }
  | { type: 'pick'; hour: number }
  | { type: 'follow' }
  | { type: 'close' };

export function reduceDrill(state: DrillState, action: DrillAction): DrillState {
  switch (action.type) {
    case 'open': {
      const follow = action.playhead != null && (action.playing || Math.abs(action.hour - action.playhead) <= FOLLOW_NEAR_HOURS);
      return { breakdown: action.breakdown, follow, hour: follow ? null : Math.floor(action.hour) };
    }
    case 'pick': return state.breakdown ? { ...state, follow: false, hour: Math.floor(action.hour) } : state;
    case 'follow': return state.breakdown ? { ...state, follow: true, hour: null } : state;
    case 'close': return CLOSED;
  }
}

export function drillHour(state: DrillState, playhead: number | null): number | null {
  return state.follow ? playhead : state.hour;
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

export function pinnedBreakdown(source: WidgetSpec, breakdown: Breakdown, trackId: string): WidgetSpec {
  const base = createWidget(source.kind, trackId);
  return { ...base, campaign: source.campaign, trackIds: [...source.trackIds], countries: source.countries, time: source.time,
    contentScale: source.contentScale, title: breakdownTitle(source.title, breakdown.label),
    options: { ...source.options, show: breakdown.id } };
}

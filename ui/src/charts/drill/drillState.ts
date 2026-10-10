import { createWidget } from '../../dashboard/model.ts';
import type { WidgetSpec } from '../../dashboard/model';
import type { OptionField } from '../../dashboard/options';

/** Drilling into what a value is made of: one pattern for every widget whose value
 * has parts. The breakdown follows playback until the user picks a day. */

export interface Breakdown { id: string; label: string }
export interface DrillState { breakdown: string | null; follow: boolean; hour: number | null }
export const CLOSED: DrillState = { breakdown: null, follow: true, hour: null };

/** Opening within this many hours (45 days) of the playhead follows playback. */
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

/** The hour the breakdown shows: the playhead while following, else the picked day. */
export function drillHour(state: DrillState, playhead: number | null): number | null {
  return state.follow ? playhead : state.hour;
}

/** The interaction setting: where a breakdown opens. */
export const DRILL_OPTION: OptionField = {
  key: 'drill', section: 'interaction', label: 'Breakdown opens', hint: 'Clicking a value shows what it is made of',
  kind: 'choice', default: 'inline', choices: [{ value: 'inline', label: 'In the widget' }, { value: 'popout', label: 'Beside it' }],
};

/** A "Show" setting listing the widget's own view and each breakdown, so a pinned
 * breakdown is simply the same widget showing that breakdown. */
export function breakdownOption(breakdowns: readonly Breakdown[], mainLabel: string): OptionField {
  return { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'main',
    choices: [{ value: 'main', label: mainLabel }, ...breakdowns.map(item => ({ value: item.id, label: item.label }))] };
}

/** "Pin as widget": a copy of the widget, for the same campaign, countries and time,
 * that shows one breakdown as its main view. */
export function pinnedBreakdown(source: WidgetSpec, breakdown: Breakdown, trackId: string): WidgetSpec {
  const base = createWidget(source.kind, trackId);
  return { ...base, campaign: source.campaign, trackIds: [...source.trackIds], countries: source.countries, time: source.time,
    contentScale: source.contentScale, title: `${source.title}: ${breakdown.label}`.slice(0, 160),
    options: { ...source.options, show: breakdown.id } };
}

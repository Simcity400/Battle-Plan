import type { GatedTime } from '../../dashboard/regionTime';
import { CLOSED, pinnedBreakdown, reduceDrill, type PinnedSpec } from '../drill/drillState.ts';
import type { DrillAction, DrillState } from '../drill/drillState';
import type { CountryPartsSelection } from './countryPartsDrillData';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';

export const UNATTRIBUTED_PART = { id: '', label: 'Other' };

export function partsRaceDrillActors(tag: string | undefined, choices: readonly string[] | undefined,
  fallback: () => string | null): readonly string[] {
  if (choices !== undefined) return [...new Set(choices)];
  const country = tag ?? fallback();
  return country ? [country] : [];
}

export interface PartsRaceDrillState extends DrillState {
  choices: { key: string; tags: readonly string[] } | null;
  selection?: CountryPartsSelection;
  owner?: unknown;
  pickOwner?: unknown;
}
export const CLOSED_PARTS_RACE: PartsRaceDrillState = { ...CLOSED, choices: null };
export type PartsRaceDrillAction = DrillAction
  | { type: 'actors'; key: string; tags: readonly string[]; hour: number }
  | { type: 'choose'; key: string; tag: string; hour: number };
export type NativePartsRaceDrillAction = PartsRaceDrillAction & { selection?: CountryPartsSelection; owner?: unknown; pickOwner?: unknown };

export function reducePartsRaceDrill(state: PartsRaceDrillState, action: NativePartsRaceDrillAction): PartsRaceDrillState {
  if (action.type === 'actors') return { ...CLOSED, spec: false, follow: false, hour: Math.floor(action.hour),
    choices: { key: action.key, tags: action.tags }, ...(action.owner ? { owner: action.owner } : {}),
    ...(action.pickOwner ? { pickOwner: action.pickOwner } : {}) };
  if (action.type === 'choose') {
    if (!state.choices || state.choices.key !== action.key || state.hour !== action.hour || !state.choices.tags.includes(action.tag)) return state;
    return { ...reduceDrill(state, { type: 'open', breakdown: action.tag, hour: action.hour, follow: false }), choices: null,
      ...(action.selection ? { selection: action.selection } : {}), ...(action.owner ? { owner: action.owner } : {}) };
  }
  if (state.choices && (action.type === 'follow' || action.type === 'pick')) return state;
  const selection = action.selection ?? (action.type === 'open' ? undefined : state.selection);
  return { ...reduceDrill(state, action), choices: null,
    ...(action.type !== 'close' && selection ? { selection } : {}),
    ...(action.type !== 'close' && (action.owner ?? state.owner) ? { owner: action.owner ?? state.owner } : {}) };
}

export function pinPartsRace(source: WidgetSpec, track: Pick<BoundTrack, 'id'>, tag: string, time: GatedTime, label: string): PinnedSpec {
  return { ...pinnedBreakdown(source, { id: 'breakdown', label }, track.id, time), countries: { kind: 'tags', tags: [tag] } };
}

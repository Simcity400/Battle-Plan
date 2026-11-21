import { contentTitle } from './contentTitles.ts';
import type { ConstructionStateName, ConstructionType, MissingSpan, SpendingGap } from '../generated/dashboard';



const int = (value: unknown) => Number.isSafeInteger(value);
const optionalText = (value: unknown) => value == null || typeof value === 'string';

export const isConstructionType = (type: ConstructionType) => typeof type === 'object' && type != null && int(type.id)
  && [type.token, type.name, type.icon].every(optionalText);

export const isStateName = (state: ConstructionStateName) => typeof state === 'object' && state != null && int(state.id) && optionalText(state.name);

export const isMissingSpan = (span: MissingSpan) => typeof span === 'object' && span != null && int(span.firstHour) && int(span.lastHour)
  && span.firstHour <= span.lastHour && typeof span.reason === 'string';

export function typeName(type: ConstructionType | undefined): string {
  return contentTitle(type?.name, type?.token, 'Building name unavailable');
}

export type StateNameAt = (state: number) => { name: string | null; missing: string | null };

export function stateNamesOf(states: readonly ConstructionStateName[]): StateNameAt {
  const names = new Map(states.map(state => [state.id, state.name]));
  return state => {
    const name = names.get(state);
    return name != null ? { name, missing: null } : { name: null, missing: names.has(state) ? 'no name in the recorded content' : 'not named by the reply' };
  };
}

export function placeName(place: { state: number | null; toState?: number | null; province?: number | null; railway: boolean }, stateAt: StateNameAt): { label: string; missing?: string } {
  const missing = new Set<string>();
  const stateName = (state: number | null | undefined) => {
    if (state == null) return null;
    const name = stateAt(state);
    if (name.missing) missing.add(`State ${state}: ${name.missing}`);
    return name.name ?? `State ${state}`;
  };
  const from = stateName(place.state);
  let label: string;
  if (place.railway) {
    const to = place.toState === place.state ? from : stateName(place.toState);
    label = from != null && to != null && place.state !== place.toState ? `${from} to ${to}` : from ?? to ?? 'an unknown route';
  } else label = from ?? (place.province != null ? `province ${place.province}` : 'an unknown place');
  return { label, missing: [...missing].join(' ') || undefined };
}

const SPENDING_GAPS: Readonly<Record<SpendingGap, true>> = {
  notRecorded: true, linkedOutput: true, inexactOutput: true, observationRestart: true, finalOutputMissing: true, ownerUnknown: true,
};

export const isSpendingGap = (value: unknown): value is SpendingGap => typeof value === 'string' && Object.hasOwn(SPENDING_GAPS, value);

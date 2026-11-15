import { contentTitle } from './contentTitles.ts';
import type { ConstructionStateName, ConstructionType, MissingSpan, SpendingGap } from '../generated/dashboard';

/** What the construction widgets share about the recorded construction queue: the reply validation of the types both read,
 * localized building titles, place names and reply validation. */


const int = (value: unknown) => Number.isSafeInteger(value);
const optionalText = (value: unknown) => value == null || typeof value === 'string';

/** A building type as a reply lists it. */
export const isConstructionType = (type: ConstructionType) => typeof type === 'object' && type != null && int(type.id)
  && [type.token, type.name, type.icon].every(optionalText);

/** A state name as a reply lists it. */
export const isStateName = (state: ConstructionStateName) => typeof state === 'object' && state != null && int(state.id) && optionalText(state.name);

/** A span of hours the recording does not hold, as a reply lists it. */
export const isMissingSpan = (span: MissingSpan) => typeof span === 'object' && span != null && int(span.firstHour) && int(span.lastHour)
  && span.firstHour <= span.lastHour && typeof span.reason === 'string';

/** A localized building title, preserving empty text and marking missing names. */
export function typeName(type: ConstructionType | undefined): string {
  return contentTitle(type?.name, type?.token, 'Building name unavailable');
}

/** A state's name, or why it has none. */
export type StateNameAt = (state: number) => { name: string | null; missing: string | null };

/** The names a reply lists, by state; a state it does not name is missing for that reason. */
export function stateNamesOf(states: readonly ConstructionStateName[]): StateNameAt {
  const names = new Map(states.map(state => [state.id, state.name]));
  return state => {
    const name = names.get(state);
    return name != null ? { name, missing: null } : { name: null, missing: names.has(state) ? 'no name in the recorded content' : 'not named by the reply' };
  };
}

/** Where a project is: its state, a railway from one state to another, else its
 * province, else a place the recording does not name. */
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

/** Every reason a stretch of a project's output is not counted. */
const SPENDING_GAPS: Readonly<Record<SpendingGap, true>> = {
  notRecorded: true, linkedOutput: true, inexactOutput: true, observationRestart: true, finalOutputMissing: true, ownerUnknown: true,
};

/** A reason a reply may give for an uncounted stretch. */
export const isSpendingGap = (value: unknown): value is SpendingGap => typeof value === 'string' && Object.hasOwn(SPENDING_GAPS, value);

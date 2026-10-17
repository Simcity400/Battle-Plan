import { contentTitle } from './contentTitles.ts';
import type { BuildingInfo, SpendingGap, StateName } from '../generated/dashboard';
import type { StateAt, StateLabel } from './stateNamesData';

/** What the construction widgets share about the recorded construction queue:
 * localized building titles, dated place names and reply validation. */

const int = (value: unknown) => Number.isSafeInteger(value);

/** A building type as a reply lists it. */
export const isBuildingInfo = (type: BuildingInfo) => typeof type === 'object' && type != null && int(type.id)
  && [type.token, type.name, type.icon].every(value => value == null || typeof value === 'string');

/** A state name as a reply lists it. */
export const isStateName = (state: StateName) => typeof state === 'object' && state != null && int(state.id)
  && (state.name == null || typeof state.name === 'string');

/** A localized building title, preserving empty text and marking missing names. */
export function typeName(type: BuildingInfo | undefined): string {
  return contentTitle(type?.name, type?.token, 'Building name unavailable');
}

export type StateNameAt = (hour: number, state: number) => StateLabel;

/** One batched shared place-names request for each distinct state at the shown
 * hour. A route's endpoints use the same read, without decoding a branch again. */
export function stateNameRequests(places: readonly { state: number | null; toState?: number | null }[], hour: number | null): StateAt[] {
  if (hour == null) return [];
  const states = new Set<number>();
  for (const place of places) for (const state of [place.state, place.toState]) if (state != null) states.add(state);
  return [...states].map(state => ({ state, hour }));
}

/** Where a project is: its state, a railway from one state to another, else its
 * province, else a place the recording does not name. */
export function placeName(place: { state: number | null; toState?: number | null; province?: number | null; railway: boolean }, hour: number, stateAt: StateNameAt): { label: string; missing?: string } {
  const missing = new Set<string>();
  const stateName = (state: number | null | undefined) => {
    if (state == null) return null;
    const name = stateAt(hour, state);
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

/** Every reason a stretch of a project's progress is not counted. */
const SPENDING_GAPS: Readonly<Record<SpendingGap, true>> = {
  notRecorded: true, readFailed: true, linkedOutput: true, inexactOutput: true,
  observationRestart: true, finalOutputMissing: true, ownerUnknown: true, preparing: true,
};

/** A reason a reply may give for an uncounted stretch. */
export const isSpendingGap = (value: unknown): value is SpendingGap => typeof value === 'string' && Object.hasOwn(SPENDING_GAPS, value);

import { contentTitle } from './contentTitles.ts';
import type { BuildingInfo, SpendingGap, StateName } from '../generated/dashboard';
import type { StateAt, StateLabel } from './stateNamesData';

/** What the construction widgets share about the recorded construction queue: how
 * a building type and a dated place are named (from recorded content and native
 * name reads, else the recording's own token or number) and the reply checks every construction
 * product needs. */

/** How often the queue is read, for a carried value's note. */

const int = (value: unknown) => Number.isSafeInteger(value);

/** A building type as a reply lists it. */
export const isBuildingInfo = (type: BuildingInfo) => typeof type === 'object' && type != null && int(type.id)
  && [type.token, type.name, type.icon].every(value => value == null || typeof value === 'string');

/** A state name as a reply lists it. */
export const isStateName = (state: StateName) => typeof state === 'object' && state != null && int(state.id)
  && (state.name == null || typeof state.name === 'string');

/** A type's name as the game gives it, else its script name, else its recorded id. */
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
    label = from != null && to != null && place.state !== place.toState ? `${from} to ${to}` : from ?? to ?? 'an unrecorded route';
  } else label = from ?? (place.province != null ? `province ${place.province}` : 'an unrecorded place');
  return { label, missing: [...missing].join(' ') || undefined };
}

/** Why a stretch of a project's progress is not counted, as the tooltip names it. */
export const GAP_LABELS: Readonly<Record<SpendingGap, string>> = {
  notRecorded: 'Factory output not recorded',
  readFailed: 'An output read failed',
  linkedOutput: 'Factory output shared with another country',
  inexactOutput: 'Instant progress or an unknown factory share',
  observationRestart: 'Output observation changed',
  finalOutputMissing: 'A line’s final output was not recorded',
  ownerUnknown: 'Output without a recorded country or target',
};

import type { BuildingInfo, StateName } from '../generated/dashboard';

/** What the construction widgets share about the recorded construction queue: how
 * a building type and a place are named (from content, else the recording's own
 * token or number, never a fixed list) and the reply checks every construction
 * product needs. */

/** How often the queue is read, for a carried value's note. */
export const CONSTRUCTION_CADENCE = 'the construction queue is read once a day.';

const int = (value: unknown) => Number.isSafeInteger(value);

/** A building type as a reply lists it. */
export const isBuildingInfo = (type: BuildingInfo) => typeof type === 'object' && type != null && int(type.id)
  && [type.token, type.name, type.icon].every(value => value == null || typeof value === 'string');

/** A state name as a reply lists it. */
export const isStateName = (state: StateName) => typeof state === 'object' && state != null && int(state.id)
  && (state.name == null || typeof state.name === 'string');

/** A type's name as the game gives it, else its script name, else its recorded id. */
export function typeName(type: BuildingInfo | undefined): string {
  return type?.name ?? type?.token ?? (type ? `Building type ${type.id}` : 'Unknown building');
}

/** A state's name as the game gives it, else its number. */
export function stateName(states: readonly StateName[], state: number | null): string | null {
  if (state == null) return null;
  return states.find(item => item.id === state)?.name ?? `State ${state}`;
}

/** Where a project is: its state, a railway from one state to another, else its
 * province, else a place the recording does not name. */
export function placeName(states: readonly StateName[], place: { state: number | null; toState?: number | null; province?: number | null; railway: boolean }): string {
  const from = stateName(states, place.state);
  if (place.railway) {
    const to = stateName(states, place.toState ?? null);
    return from && to && from !== to ? `${from} to ${to}` : from ?? to ?? 'an unrecorded route';
  }
  return from ?? (place.province != null ? `province ${place.province}` : 'an unrecorded place');
}

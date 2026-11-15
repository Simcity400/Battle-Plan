import type { BoundTrack } from './model';
import { setToken } from './options.ts';

/** An air theatre and the strategic regions it holds at the hour it was read. */
export interface AirTheatre { id: string; name: string; regions: readonly string[] }

/** Air theatres at an hour: the theatres, or why they are not shown. */
export interface AirTheatresAt { theatres: readonly AirTheatre[] | null; unavailable: string | null }

export const AIR_THEATRES_NOT_SERVED = 'Air theatres are not shown yet: they are read with the air missions, which are not ported yet.';

/** The one way a widget reads air theatres at `hour` of `track`. Theatre membership is the air missions family's
 * read, which is not on the query layer yet: until it is, every hour says so by name, and a widget filtering by a
 * theatre shows that instead of a guess. */
export function useAirTheatres(_track: BoundTrack | undefined, _hour: number | null, _active: boolean): AirTheatresAt {
  return { theatres: null, unavailable: AIR_THEATRES_NOT_SERVED };
}

/** The regions of the chosen theatres; null keeps every region. */
export function theatreRegions(theatres: readonly AirTheatre[] | null, chosen: readonly string[]): Set<string> | null {
  if (!chosen.length) return null;
  return new Set((theatres ?? []).filter(theatre => chosen.includes(theatre.id)).flatMap(theatre => theatre.regions));
}

/** Theatre choices at that hour, keeping a chosen theatre absent then selectable under its id. */
export function theatreChoices(theatres: readonly AirTheatre[] | null, chosen: readonly string[]) {
  const listed = theatres ?? [];
  return [...listed.map(theatre => ({ value: setToken(theatre.id), label: theatre.name })),
    ...chosen.filter(id => !listed.some(theatre => theatre.id === id)).map(id => ({ value: setToken(id), label: `${id} · not at this date` }))];
}

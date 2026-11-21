import type { BoundTrack } from './model';
import { setToken } from './options.ts';

export interface AirTheatre { id: string; name: string; regions: readonly string[] }

export interface AirTheatresAt { theatres: readonly AirTheatre[] | null; unavailable: string | null }

export const AIR_THEATRES_NOT_SERVED = 'Air theatres are not shown yet: they are read with the air missions, which are not ported yet.';

export function useAirTheatres(_track: BoundTrack | undefined, _hour: number | null, _active: boolean): AirTheatresAt {
  return { theatres: null, unavailable: AIR_THEATRES_NOT_SERVED };
}

export function theatreRegions(theatres: readonly AirTheatre[] | null, chosen: readonly string[]): Set<string> | null {
  if (!chosen.length) return null;
  return new Set((theatres ?? []).filter(theatre => chosen.includes(theatre.id)).flatMap(theatre => theatre.regions));
}

export function theatreChoices(theatres: readonly AirTheatre[] | null, chosen: readonly string[]) {
  const listed = theatres ?? [];
  return [...listed.map(theatre => ({ value: setToken(theatre.id), label: theatre.name })),
    ...chosen.filter(id => !listed.some(theatre => theatre.id === id)).map(id => ({ value: setToken(id), label: `${id} · not at this date` }))];
}

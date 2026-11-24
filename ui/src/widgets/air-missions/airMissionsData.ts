import { setToken } from '../../dashboard/options.ts';
import type { AirMissionsReply, AirMissionTheatre } from '../../generated/dashboard';

const text = (value: unknown): value is string => typeof value === 'string';
const optionalText = (value: unknown) => value === null || text(value);
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(text);
const hour = (value: unknown) => value === null || Number.isSafeInteger(value);
const sprite = (value: unknown) => value === null || !!value && typeof value === 'object'
  && text((value as { key: unknown }).key) && Number.isSafeInteger((value as { frame: unknown }).frame) && (value as { frame: number }).frame >= 0
  && ((value as { recorded?: unknown }).recorded === undefined || text((value as { recorded: unknown }).recorded) && (value as { recorded: string }).recorded !== '');

export function validateAirMissions(value: unknown): AirMissionsReply {
  const reply = value as AirMissionsReply;
  if (!reply || typeof reply !== 'object' || !hour(reply.readHour) || !optionalText(reply.missing) || !optionalText(reply.theatresMissing) || !strings(reply.warnings)
    || !Array.isArray(reply.missions) || !reply.missions.every(mission => mission && text(mission.id) && text(mission.name) && sprite(mission.icon))
    || !Array.isArray(reply.regions) || !reply.regions.every(region => region && text(region.region) && optionalText(region.name) && optionalText(region.missing) && strings(region.missions))
    || !(reply.theatres === null || Array.isArray(reply.theatres) && reply.theatres.every(theatre => theatre && text(theatre.id) && text(theatre.name) && strings(theatre.regions))))
    throw new Error('The air missions reply is malformed.');
  return reply;
}

export function theatreRegions(theatres: readonly AirMissionTheatre[] | null | undefined, chosen: readonly string[]): Set<string> | null {
  if (!chosen.length) return null;
  return new Set((theatres ?? []).filter(theatre => chosen.includes(theatre.id)).flatMap(theatre => theatre.regions));
}

export function choicesAt(listed: readonly { id: string; name: string }[], chosen: readonly string[]) {
  return [...listed.map(item => ({ value: setToken(item.id), label: item.name })),
    ...chosen.filter(id => !listed.some(item => item.id === id)).map(id => ({ value: setToken(id), label: `${id} · not at this date` }))];
}

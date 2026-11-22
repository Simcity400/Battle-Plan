import type { TooltipContent } from '../../charts/tooltip';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { setToken } from '../../dashboard/options.ts';
import type { AirMissionChoice, AirMissionsReply, AirMissionTheatre, AirMissionWingsReply } from '../../generated/dashboard';

export const NOT_FLOWN = 'No wing in this region flies this mission';

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

export function validateAirWings(value: unknown): AirMissionWingsReply {
  const reply = value as AirMissionWingsReply;
  if (!reply || typeof reply !== 'object' || !hour(reply.readHour) || !Number.isSafeInteger(reply.region) || !optionalText(reply.missing) || !strings(reply.warnings)
    || !Array.isArray(reply.wings) || !reply.wings.every(wing => wing && text(wing.id) && optionalText(wing.name) && optionalText(wing.type)
      && strings(wing.missions) && optionalText(wing.efficiency) && strings(wing.refusals)))
    throw new Error('The air wings reply is malformed.');
  return reply;
}

export interface MissionRow { key: string; region: string; name: string; nameMissing: string | null; missions: ReadonlySet<string> }

export const regionLabel = (id: string, name: string | null) => name ?? `Region ${id}`;

export function missionRows(reply: AirMissionsReply): MissionRow[] {
  return reply.regions.map(region => ({ key: region.region, region: region.region, name: regionLabel(region.region, region.name),
    nameMissing: region.missing, missions: new Set(region.missions) }));
}

export function regionRows(rows: readonly MissionRow[], regions: ReadonlySet<string> | null): MissionRow[] {
  return regions ? rows.filter(row => regions.has(row.region)) : [...rows];
}

export function theatreRegions(theatres: readonly AirMissionTheatre[] | null | undefined, chosen: readonly string[]): Set<string> | null {
  if (!chosen.length) return null;
  return new Set((theatres ?? []).filter(theatre => chosen.includes(theatre.id)).flatMap(theatre => theatre.regions));
}

export function shownMissions(missions: readonly AirMissionChoice[], rows: readonly MissionRow[], chosen: readonly string[]): AirMissionChoice[] {
  const present = new Set(rows.flatMap(row => [...row.missions]));
  return missions.filter(mission => present.has(mission.id) && (!chosen.length || chosen.includes(mission.id)));
}

export function missionValues(values: LiveValues<number>, rows: readonly MissionRow[]): LiveValues<number> {
  const notFlown = flownBy(rows);
  return { get: key => values.get(key), missing: key => notFlown(key) ? NOT_FLOWN : values.missing(key) };
}

export function flownBy(rows: readonly MissionRow[]): (key: string) => boolean {
  const flown = new Map(rows.map(row => [row.key, row.missions]));
  return key => {
    const cut = key.lastIndexOf('\u0000'), column = key.slice(cut + 1);
    return column.startsWith('m') && !flown.get(key.slice(0, cut))?.has(column.slice(1));
  };
}

const count = (value: number) => value.toLocaleString('en-US');

export function missionTooltip(row: MissionRow, missions: readonly AirMissionChoice[], values: LiveValues<number>, readDate: string | null): TooltipContent {
  const wings = values.get(liveKey(row.key, 'wings'));
  return {
    title: row.name,
    rows: [
      ...missions.filter(mission => row.missions.has(mission.id)).map(mission => {
        const planes = values.get(liveKey(row.key, `m${mission.id}`));
        return { label: mission.name, lookupKey: mission.id,
          value: planes === undefined ? values.missing(liveKey(row.key, `m${mission.id}`)) : `${count(planes)} ${planes === 1 ? 'plane' : 'planes'}` };
      }),
      { label: 'Wings', value: wings === undefined ? values.missing(liveKey(row.key, 'wings')) : count(wings) },
      ...row.nameMissing ? [{ label: 'Name', value: row.nameMissing }] : [],
      ...readDate ? [{ label: 'As of', value: readDate }] : [],
    ],
  };
}

export function efficiencyText(raw: string | null): string {
  if (raw === null) return '—';
  const value = Number(raw) / 1000;
  return `${value.toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
}


export function choicesAt(listed: readonly { id: string; name: string }[], chosen: readonly string[]) {
  return [...listed.map(item => ({ value: setToken(item.id), label: item.name })),
    ...chosen.filter(id => !listed.some(item => item.id === id)).map(id => ({ value: setToken(id), label: `${id} · not at this date` }))];
}

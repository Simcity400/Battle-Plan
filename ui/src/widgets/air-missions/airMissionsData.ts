import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedWindow } from '../../dashboard/keyedHistory';
import type { TooltipContent } from '../../charts/tooltip';
import { optionalKey } from '../../dashboard/spriteArtwork.ts';
import type { SpriteFrame } from '../../dashboard/spriteArtwork';

export interface Choice { id: string; name: string }
/** A mission column: its bit, its name and the mission strip frame the game draws for it. */
export interface MissionChoice extends Choice { icon: SpriteFrame | null }
export interface AirTheatre extends Choice { regions: readonly string[] }
export interface AirMissionsReply { window: KeyedWindow; missions: readonly MissionChoice[]; regions: readonly Choice[]; theatres?: readonly AirTheatre[]; warnings: readonly string[] }
export interface AirWing { id: string; name: string | null; type: string | null; planes: number; missions: readonly string[]; efficiency: string | null; refusals: readonly string[] }
export interface AirWingsReply { hour: number; readHour: number | null; region: number; wings: readonly AirWing[]; warnings: readonly string[] }

const isChoice = (value: unknown): value is Choice => !!value && typeof value === 'object'
  && typeof (value as Choice).id === 'string' && typeof (value as Choice).name === 'string';
const isSprite = (value: unknown) => value === null || !!value && typeof value === 'object'
  && typeof (value as SpriteFrame).key === 'string' && Number.isSafeInteger((value as SpriteFrame).frame) && (value as SpriteFrame).frame >= 0
  && optionalKey((value as SpriteFrame).recorded);
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');

export function validateAirMissions(value: unknown): AirMissionsReply {
  const reply = value as AirMissionsReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.missions) || !reply.missions.every(mission => isChoice(mission) && isSprite((mission as MissionChoice).icon))
    || !Array.isArray(reply.regions) || !reply.regions.every(isChoice) || !strings(reply.warnings)
    || reply.theatres !== undefined && !(Array.isArray(reply.theatres) && reply.theatres.every(theatre => isChoice(theatre) && strings((theatre as AirTheatre).regions))))
    throw new Error('The air missions reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

export function validateAirWings(value: unknown): AirWingsReply {
  const reply = value as AirWingsReply;
  if (!reply || typeof reply !== 'object' || !Number.isSafeInteger(reply.hour) || !(reply.readHour === null || Number.isSafeInteger(reply.readHour))
    || !Number.isSafeInteger(reply.region) || !strings(reply.warnings) || !Array.isArray(reply.wings)
    || !reply.wings.every(wing => wing && typeof wing.id === 'string' && (wing.name === null || typeof wing.name === 'string')
      && (wing.type === null || typeof wing.type === 'string') && Number.isSafeInteger(wing.planes) && strings(wing.missions)
      && (wing.efficiency === null || typeof wing.efficiency === 'string') && strings(wing.refusals)))
    throw new Error('The air wings reply is malformed.');
  return reply;
}

/** One region's assignments at an hour: planes per mission (a wing on several missions
 * counts in each) and its wings counted once. */
export interface MissionRow { key: string; region: string; name: string; planes: ReadonlyMap<string, bigint>; wings: bigint | null }

/** The country's regions with wings assigned at `hour`; null when not recorded then. */
export function missionRowsAt(reply: AirMissionsReply, tag: string, hour: number): { rows: MissionRow[]; readHour: number | undefined } | null {
  if (!keyedKnown(reply.window, hour)) return null;
  const names = new Map(reply.regions.map(region => [region.id, region.name]));
  const rows = new Map<string, { region: string; name: string; planes: Map<string, bigint>; wings: bigint | null }>();
  for (const series of reply.window.series) {
    const [country, region, field] = series.name.split('|');
    if (countryPart(country)?.tag !== tag || !field) continue;
    const run = runAtHour(series.runs, hour);
    if (!run) continue;
    const row = rows.get(region) ?? { region, name: names.get(region) ?? 'Region name unavailable', planes: new Map(), wings: null };
    if (field === 'wings') row.wings = BigInt(run[2]);
    else if (field.startsWith('m')) row.planes.set(field.slice(1), BigInt(run[2]));
    else continue;
    rows.set(region, row);
  }
  return { rows: [...rows.values()].map(row => ({ key: row.region, ...row })), readHour: keyedRead(reply.window, hour) };
}

/** Missions any shown row has an entry for, in the recording's mission order. */
export function shownMissions(missions: readonly MissionChoice[], rows: readonly MissionRow[], chosen: readonly string[]): MissionChoice[] {
  const present = new Set(rows.flatMap(row => [...row.planes.keys()]));
  return missions.filter(mission => present.has(mission.id) && (!chosen.length || chosen.includes(mission.id)));
}

/** Rows in the chosen regions (null keeps all). */
export function regionRows(rows: readonly MissionRow[], regions: ReadonlySet<string> | null): MissionRow[] {
  return regions ? rows.filter(row => regions.has(row.region)) : [...rows];
}

/** The regions of the chosen theatres; null keeps every region. */
export function theatreRegions(theatres: readonly AirTheatre[] | undefined, chosen: readonly string[]): Set<string> | null {
  if (!chosen.length) return null;
  return new Set((theatres ?? []).filter(theatre => chosen.includes(theatre.id)).flatMap(theatre => theatre.regions));
}

const count = (value: bigint) => value.toLocaleString('en-US');

export function missionTooltip(row: MissionRow, missions: readonly Choice[], readDate: string | null): TooltipContent {
  return {
    title: row.name,
    rows: [
      ...missions.filter(mission => row.planes.has(mission.id)).map(mission => ({ label: mission.name, lookupKey: mission.id,
        value: `${count(row.planes.get(mission.id)!)} ${row.planes.get(mission.id) === 1n ? 'plane' : 'planes'}` })),
      { label: 'Wings', value: row.wings === null ? 'Wings unknown' : count(row.wings) },
      ...readDate ? [{ label: 'As of', value: readDate }] : [],
    ],
  };
}

/** A fixed-point efficiency on the game's percent scale. */
export function efficiencyText(raw: string | null): string {
  if (raw === null) return '—';
  const value = Number(raw) / 1000;
  return `${value.toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
}

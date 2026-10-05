import type { RaidEpisode, RaidRun, RaidsReply, RaidTypeInfo } from '../generated/dashboard';
import { drawableIcon } from './artwork.ts';
import { validSpans } from './spans.ts';

/** Both raid widgets and the picker use the same product and content identities. */
export const RAIDS_AVAILABILITY = { product: 'raids', params: { view: 'catalogue', tags: [], top: null } } as const;
const integer = (value: unknown) => Number.isSafeInteger(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const optionalInteger = (value: unknown) => value === null || integer(value);
const optionalBoolean = (value: unknown) => value === null || typeof value === 'boolean';

export function validateRaidsReply(value: unknown): RaidsReply {
  const reply = value as RaidsReply;
  const ok = reply && typeof reply.branchId === 'string' && optionalBoolean(reply.available) && text(reply.unavailable)
    && Array.isArray(reply.types) && reply.types.every(type => typeof type.token === 'string' && text(type.name) && text(type.missing))
    && new Set(reply.types.map(type => type.token)).size === reply.types.length
    && Array.isArray(reply.artwork) && reply.artwork.every(art => typeof art.key === 'string' && text(art.image) && text(art.missing))
    && new Set(reply.artwork.map(art => art.key)).size === reply.artwork.length
    && Array.isArray(reply.readHours) && reply.readHours.every(integer) && reply.readHours.every((hour, index) => !index || reply.readHours[index - 1] < hour)
    && validSpans(reply.unobserved) && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.episodes) && reply.episodes.every(episode => typeof episode.id === 'string'
      && episode.start && integer(episode.start.hour) && validSpans(episode.runs)
      && Array.isArray(episode.preparation) && episode.preparation.every((read, index) => Array.isArray(read) && read.length === 2
        && read.every(integer) && (index === 0 || episode.preparation[index - 1][0] < read[0]))
      && episode.runs.every(run => integer(run.readHour) && run.readHour <= run.firstHour
        && [run.raider, run.victim, run.type, run.targetName, run.phaseName, run.outcomeName, run.icon, run.iconMissing].every(text)
        && [run.province, run.state].every(optionalInteger) && [run.phase, run.outcome].every(integer)
        && [run.launched, run.succeeded].every(optionalBoolean)))
    && new Set(reply.episodes.map(episode => episode.id)).size === reply.episodes.length
    && Array.isArray(reply.series) && reply.series.every(series => typeof series.tag === 'string' && validSpans(series.runs) && validSpans(series.segments)
      && (series.membership === null || validSpans(series.membership))
      && series.runs.every(run => integer(run.readHour) && run.readHour <= run.firstHour && integer(run.launched) && integer(run.succeeded)
        && run.launched >= 0 && run.succeeded >= 0))
    && new Set(reply.series.map(series => series.tag)).size === reply.series.length;
  if (!ok) throw new Error('The raids reply is malformed.');
  return { ...reply, artwork: reply.artwork.map(art => ({ ...art, image: drawableIcon(art.image) })) };
}

export function raidType(types: readonly RaidTypeInfo[], run: RaidRun): RaidTypeInfo | null {
  return types.find(type => type.token === run.type) ?? null;
}
export function raidTypeName(type: RaidTypeInfo | null): string { return type?.name ?? (type ? `Unknown name (${type.token})` : 'Raid type not known'); }
/** A content key keeps equally localised type names distinguishable in filters. */
export function raidTypeChoices(types: readonly RaidTypeInfo[]): { token: string; label: string }[] {
  const counts = new Map<string, number>();
  for (const type of types) { const name = raidTypeName(type); counts.set(name, (counts.get(name) ?? 0) + 1); }
  return types.map(type => { const name = raidTypeName(type); return { token: type.token, label: counts.get(name)! > 1 ? `${name} (${type.token})` : name }; });
}
export function raidIcon(data: RaidsReply, run: RaidRun): string | null {
  return run.icon === null ? null : data.artwork.find(art => art.key === run.icon)?.image ?? null;
}
export function raidPlace(run: RaidRun): string {
  return run.targetName ?? (run.province !== null ? `Province ${run.province}` : run.state !== null ? `State ${run.state}` : 'Target not known');
}
export interface RaidAt { episode: RaidEpisode; latest: RaidRun; current: RaidRun | null; hour: number }

/** The raid's own last observation, even when other countries missed that read. */
export function raidReadAt(episode: RaidEpisode, hour: number): { readHour: number; hours: number } | null {
  const reads = episode.preparation;
  let found: { readHour: number; hours: number } | null = null;
  for (let low = 0, high = reads.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (reads[middle][0] <= hour) { found = { readHour: reads[middle][0], hours: reads[middle][1] }; low = middle + 1; }
    else high = middle - 1;
  }
  return found;
}

/** Preparation belongs to its exact read, never an earlier hour across a gap. */
export function raidPreparationRead(at: RaidAt): { readHour: number; hours: number } | null {
  if (!at.current) return null;
  const hour = Math.floor(at.hour);
  const read = raidReadAt(at.episode, hour);
  return read?.readHour === hour ? read : null;
}

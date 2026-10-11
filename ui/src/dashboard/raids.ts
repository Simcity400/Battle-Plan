import { contentTitle } from './contentTitles.ts';
import type { RaidEpisode, RaidPreparationRun, RaidRun, RaidsReply, RaidTypeInfo } from '../generated/dashboard';
import { drawableIcon } from './artwork.ts';
import { isOrderedSpans } from '../charts/series.ts';
import { validCountryIdentityRuns } from './countryIdentity.ts';

/** Both raid widgets and the picker use the same product and content identities. */
export const RAIDS_AVAILABILITY = { product: 'raids', params: { view: 'catalogue', tags: [], top: null } } as const;
const integer = (value: unknown) => Number.isSafeInteger(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const optionalInteger = (value: unknown) => value === null || integer(value);
const optionalBoolean = (value: unknown) => value === null || typeof value === 'boolean';
const preparationRuns = (runs: unknown): runs is RaidPreparationRun[] => Array.isArray(runs) && runs.every((run, index) =>
  Array.isArray(run) && run.length === 4 && run.every(integer) && run[0] <= run[1]
  && integer(run[2] + (run[1] - run[0]) * run[3]) && (index === 0 || runs[index - 1][1] < run[0]));

export function validateRaidsReply(value: unknown): RaidsReply {
  const reply = value as RaidsReply;
  const ok = reply && typeof reply.branchId === 'string' && optionalBoolean(reply.available) && text(reply.unavailable)
    && [reply.phasePreparing, reply.phaseEnded, reply.outcomeNone].every(integer)
    && Array.isArray(reply.types) && reply.types.every(type => typeof type.token === 'string' && text(type.name) && text(type.missing))
    && new Set(reply.types.map(type => type.token)).size === reply.types.length
    && Array.isArray(reply.artwork) && reply.artwork.every(art => typeof art.key === 'string' && text(art.image) && text(art.missing))
    && new Set(reply.artwork.map(art => art.key)).size === reply.artwork.length
    && Array.isArray(reply.readHours) && reply.readHours.every(integer) && reply.readHours.every((hour, index) => !index || reply.readHours[index - 1] < hour)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.episodes) && reply.episodes.every(episode => typeof episode.id === 'string'
      && episode.start && integer(episode.start.hour) && isOrderedSpans(episode.runs) && preparationRuns(episode.preparation)
      && episode.runs.every(run => [run.raider, run.victim, run.type, run.targetName, run.phaseName, run.outcomeName, run.icon, run.iconMissing].every(text)
        && [run.province, run.state].every(optionalInteger) && [run.phase, run.outcome].every(integer)
        && [run.launched, run.succeeded].every(optionalBoolean)))
    && new Set(reply.episodes.map(episode => episode.id)).size === reply.episodes.length
    && Array.isArray(reply.series) && reply.series.every(series => typeof series.tag === 'string' && isOrderedSpans(series.runs) && isOrderedSpans(series.segments)
      && (series.membership === null || isOrderedSpans(series.membership)) && validCountryIdentityRuns(series.identities)
      && series.runs.every(run => integer(run.readHour) && run.readHour <= run.firstHour && integer(run.launched) && integer(run.succeeded)
        && run.launched >= 0 && run.succeeded >= 0))
    && new Set(reply.series.map(series => series.tag)).size === reply.series.length;
  if (!ok) throw new Error('The raids reply is malformed.');
  return { ...reply, artwork: reply.artwork.map(art => ({ ...art, image: drawableIcon(art.image) })) };
}

export function raidType(types: readonly RaidTypeInfo[], run: RaidRun): RaidTypeInfo | null {
  return types.find(type => type.token === run.type) ?? null;
}
export function raidTypeName(type: RaidTypeInfo | null): string { return contentTitle(type?.name, type?.token, 'Raid name unavailable'); }
/** Repeated localized titles keep distinct choices without exposing script keys. */
export function raidTypeChoices(types: readonly RaidTypeInfo[]): { token: string; label: string }[] {
  const counts = new Map<string, number>();
  for (const type of types) { const name = raidTypeName(type); counts.set(name, (counts.get(name) ?? 0) + 1); }
  const ordinals = new Map<string, number>();
  return types.map(type => {
    const name = raidTypeName(type), ordinal = (ordinals.get(name) ?? 0) + 1;
    ordinals.set(name, ordinal);
    return { token: type.token, label: counts.get(name)! > 1 ? `${name} (${ordinal})` : name };
  });
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
  const runs = episode.preparation;
  const at = Math.floor(hour);
  let found: RaidPreparationRun | null = null;
  for (let low = 0, high = runs.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (runs[middle][0] <= at) { found = runs[middle]; low = middle + 1; }
    else high = middle - 1;
  }
  if (!found) return null;
  const readHour = Math.min(at, found[1]);
  return { readHour, hours: found[2] + (readHour - found[0]) * found[3] };
}

/** Preparation belongs to its exact read, never an earlier hour across a gap. */
export function raidPreparationRead(at: RaidAt): { readHour: number; hours: number } | null {
  if (!at.current) return null;
  const hour = Math.floor(at.hour);
  const read = raidReadAt(at.episode, hour);
  return read?.readHour === hour ? read : null;
}

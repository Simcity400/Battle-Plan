import type { LogRow } from '../../charts/log/logData';
import { newestFirst } from '../../charts/log/logData.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { raidAt, raidIcon, raidPlaceId, raidPreparationText, raidStatus, raidTypeChoices, raidTypeText, startedBy } from '../../dashboard/raids.ts';
import type { RaidAt } from '../../dashboard/raids';
import type { PlaceNamesReply, RaidEpisode, RaidLogReply } from '../../generated/dashboard';

/** Chips cover only the types this branch's raids show; All types still accepts every type. */
export function raidLogTypes(data: RaidLogReply): { token: string; label: string; icon: string | null }[] {
  const used = new Map<string, RaidEpisode>();
  for (const episode of data.episodes) if (episode.raidType !== null && !used.has(episode.raidType)) used.set(episode.raidType, episode);
  const declared = data.types.filter(type => used.has(type.token));
  const undeclared = [...used.keys()].filter(token => !data.types.some(type => type.token === token)).map(token => ({ token, name: used.get(token)!.raidTypeName, missing: null }));
  return raidTypeChoices([...declared, ...undeclared]).map(choice => ({ ...choice, icon: raidIcon(data, used.get(choice.token)!) }));
}

/** The raids at `hour`, newest first, each dated by the phase it shows then: those of the chosen `types` (every type
 * when none) whose raider `matches` at that date, from `from` on when given. */
export function raidLogRows(data: Pick<RaidLogReply, 'episodes'>, hour: number, types: readonly string[], matches: (tag: string | null, hour: number) => boolean,
  from: number | null): ReadonlyArray<LogRow<RaidAt>> {
  const selected = new Set(types);
  return newestFirst(data.episodes.slice(0, startedBy(data.episodes, hour)).flatMap(episode => {
    const at = raidAt(episode, hour);
    if (!at || (selected.size && !selected.has(episode.raidType ?? '')) || !matches(episode.raider, at.since) || (from !== null && at.since < from)) return [];
    return [{ id: episode.id, key: JSON.stringify([episode.id, at.since]), hour: at.since, by: true, item: at }];
  }));
}

/** The targets to name, all at `hour` (one frame read, whatever the raids' hours): a state where the target has one,
 * else its province; every raid's, each place once. */
export function targetNameRequest(episodes: readonly RaidEpisode[], hour: number): { places: { provinceId: number; hour: number }[]; states: { stateId: number; hour: number }[] } {
  const places = new Map<number, { provinceId: number; hour: number }>(), states = new Map<number, { stateId: number; hour: number }>();
  for (let at = episodes.length - 1; at >= 0; at--) {
    const episode = episodes[at];
    if (episode.state !== null) states.set(episode.state, { stateId: episode.state, hour });
    else if (episode.province !== null) places.set(episode.province, { provinceId: episode.province, hour });
  }
  return { places: [...places.values()], states: [...states.values()] };
}

/** Target names by place, `state:<id>` or `province:<id>`, each its name or why it has none. */
export type TargetNames = ReadonlyMap<string, { name: string | null; missing: string | null }>;

export function targetNames(reply: PlaceNamesReply): TargetNames {
  const names = new Map<string, { name: string | null; missing: string | null }>();
  for (const item of reply.states) names.set(`state:${item.stateId}`, item);
  for (const item of reply.places) names.set(`province:${item.provinceId}`, item);
  return names;
}

/** A target's name, else its id and why it is not named (only its id while no names are known). */
export function targetText(episode: RaidEpisode, names?: TargetNames | null): string {
  const [kind, id] = episode.state !== null ? ['state', episode.state] as const : episode.province !== null ? ['province', episode.province] as const : [null, null];
  if (kind === null) return raidPlaceId(episode);
  const label = `${kind === 'state' ? 'State' : 'Province'} ${id}`;
  if (!names) return label;
  const found = names.get(`${kind}:${id}`);
  return found?.name ?? `${label} (${found?.missing ?? 'Name not read yet'})`;
}

export function raidDescription(data: RaidLogReply, at: RaidAt, raider: string, victim: string, target: string, hour: number): string {
  const preparation = raidPreparationText(data, at, hour);
  return `${raider} targets ${victim} at ${target} with ${raidTypeText(data, at.episode)}. ${raidStatus(data, at)}.${preparation ? ` ${preparation}.` : ''}`;
}

export function raidTooltip(data: RaidLogReply, at: RaidAt, raider: string, victim: string, target: string, hour: number): TooltipContent {
  const preparation = raidPreparationText(data, at, hour);
  return { title: raidTypeText(data, at.episode), icon: raidIcon(data, at.episode) ?? undefined, rows: [
    { label: 'Raider', value: raider }, { label: 'Target country', value: victim }, { label: 'Target', value: target },
    { label: 'Phase / outcome', value: raidStatus(data, at) },
    ...(preparation !== null ? [{ label: 'Hours prepared', value: preparation === 'Preparation not known' ? 'Not known' : preparation.replace(' h prepared', '') }] : []),
    { label: 'Seen by', value: formatDateTime(at.episode.firstHour) },
  ] };
}

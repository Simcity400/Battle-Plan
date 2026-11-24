import type { LogRow } from '../../charts/log/logData';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { raidAt, raidIcon, raidPlaceId, raidPreparationText, raidStatus, raidTypeChoices, raidTypeText, raidVictimLabel, startedBy } from '../../dashboard/raids.ts';
import type { RaidAt } from '../../dashboard/raids';
import type { PlaceNamesReply, RaidEpisode, RaidLogReply } from '../../generated/dashboard';

export function raidLogTypes(data: RaidLogReply): { token: string; label: string; icon: string | null }[] {
  const used = new Map<string, RaidEpisode>();
  for (const episode of data.episodes) if (episode.raidType !== null && !used.has(episode.raidType)) used.set(episode.raidType, episode);
  const declared = data.types.filter(type => used.has(type.token));
  const undeclared = [...used.keys()].filter(token => !data.types.some(type => type.token === token)).map(token => ({ token, name: used.get(token)!.raidTypeName, missing: null }));
  return raidTypeChoices([...declared, ...undeclared]).map(choice => ({ ...choice, icon: raidIcon(data, used.get(choice.token)!) }));
}

function compareRaidIds(a: string, b: string): number {
  const digits = /^\d+$/;
  if (digits.test(a) && digits.test(b) && a.length !== b.length) return a.length - b.length;
  return a < b ? -1 : a > b ? 1 : 0;
}

function orderRaidRows(rows: readonly LogRow<RaidAt>[]): ReadonlyArray<LogRow<RaidAt>> {
  const items = rows.map(row => {
    const high = row.item.since;
    const low = row.item.sinceExact ? high : (row.item.afterHour !== null ? row.item.afterHour + 1 : -Infinity);
    return { row, low, high };
  });
  items.sort((a, b) => a.low - b.low || a.high - b.high || compareRaidIds(a.row.id ?? '', b.row.id ?? ''));
  const clusters: { maxHigh: number; rows: LogRow<RaidAt>[] }[] = [];
  for (const entry of items) {
    const last = clusters[clusters.length - 1];
    if (last && entry.low <= last.maxHigh) {
      last.maxHigh = Math.max(last.maxHigh, entry.high);
      last.rows.push(entry.row);
    } else {
      clusters.push({ maxHigh: entry.high, rows: [entry.row] });
    }
  }
  clusters.sort((a, b) => b.maxHigh - a.maxHigh);
  const ordered: LogRow<RaidAt>[] = [];
  for (const cluster of clusters) {
    cluster.rows.sort((a, b) => compareRaidIds(a.id ?? '', b.id ?? ''));
    ordered.push(...cluster.rows);
  }
  return ordered;
}

export function raidLogRows(data: Pick<RaidLogReply, 'episodes'>, hour: number, types: readonly string[], matches: (tag: string | null, hour: number) => boolean,
  from: number | null): ReadonlyArray<LogRow<RaidAt>> {
  const selected = new Set(types);
  return orderRaidRows(data.episodes.slice(0, startedBy(data.episodes, hour)).flatMap(episode => {
    const at = raidAt(episode, hour);
    if (!at || (selected.size && !selected.has(episode.raidType ?? '')) || !matches(episode.raider, at.since) || (from !== null && at.since < from)) return [];
    return [{ id: episode.id, key: JSON.stringify([episode.id, at.since]), hour: at.since, by: !at.sinceExact, item: at }];
  }));
}

export function targetNameRequest(episodes: readonly RaidEpisode[], hour: number): { places: { provinceId: number; hour: number }[]; states: { stateId: number; hour: number }[] } {
  const places = new Map<number, { provinceId: number; hour: number }>(), states = new Map<number, { stateId: number; hour: number }>();
  for (let at = episodes.length - 1; at >= 0; at--) {
    const episode = episodes[at];
    if (episode.state !== null) states.set(episode.state, { stateId: episode.state, hour });
    else if (episode.province !== null) places.set(episode.province, { provinceId: episode.province, hour });
  }
  return { places: [...places.values()], states: [...states.values()] };
}

export type TargetNames = ReadonlyMap<string, { name: string | null; missing: string | null }>;

export function targetNames(reply: PlaceNamesReply): TargetNames {
  const names = new Map<string, { name: string | null; missing: string | null }>();
  for (const item of reply.states) names.set(`state:${item.stateId}`, item);
  for (const item of reply.places) names.set(`province:${item.provinceId}`, item);
  return names;
}

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
  const endedRow = at.episode.endedHour !== null
    ? [{ label: at.episode.endedExact ? 'Ended' : 'Ended by', value: formatDateTime(at.episode.endedHour) }]
    : at.disappeared && at.episode.disappearedHour !== null
      ? [{ label: at.episode.disappearedExact ? 'No longer seen' : 'No longer seen by', value: formatDateTime(at.episode.disappearedHour) }]
      : [];
  return { title: raidTypeText(data, at.episode), icon: raidIcon(data, at.episode) ?? undefined, rows: [
    { label: 'Raider', value: raider }, { label: raidVictimLabel(at.episode.victimSource), value: victim }, { label: 'Target', value: target },
    { label: 'Phase / outcome', value: raidStatus(data, at) },
    ...(preparation !== null ? [{ label: 'Hours prepared', value: preparation === 'Preparation not known' ? 'Not known' : preparation.replace(' h prepared', '') }] : []),
    { label: at.episode.firstHourExact ? 'First seen' : 'First seen by', value: formatDateTime(at.episode.firstHour) },
    ...endedRow,
  ] };
}

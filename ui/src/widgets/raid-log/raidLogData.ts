import type { LogRow, LogSection } from '../../charts/log/logData';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { raidAt, raidIcon, raidPlaceId, raidPreparationText, raidStatus, raidTypeChoices, raidTypeText, raidVictimLabel, raidVictimRole, startedBy } from '../../dashboard/raids.ts';
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

export interface RaidLogGroup {
  readonly key: string;
  readonly label: string | null;
  readonly unordered: boolean;
  readonly rows: ReadonlyArray<LogRow<RaidAt>>;
}

function groupRaidRows(rows: readonly LogRow<RaidAt>[]): ReadonlyArray<RaidLogGroup> {
  const items = rows.map(row => {
    const high = row.item.since;
    const low = row.item.sinceExact ? high : (row.item.afterHour !== null ? row.item.afterHour + 1 : -Infinity);
    return { row, low, high, bounded: !row.item.sinceExact };
  });
  items.sort((a, b) => b.high - a.high || b.low - a.low || compareRaidIds(a.row.id ?? '', b.row.id ?? ''));
  interface Clique { maxLow: number; minHigh: number; minLow: number; maxHigh: number; hasBounded: boolean; rows: LogRow<RaidAt>[] }
  const cliques: Clique[] = [];
  for (const entry of items) {
    const last = cliques[cliques.length - 1];
    if (last && (last.hasBounded || entry.bounded) && Math.max(last.maxLow, entry.low) <= Math.min(last.minHigh, entry.high)) {
      last.maxLow = Math.max(last.maxLow, entry.low);
      last.minHigh = Math.min(last.minHigh, entry.high);
      last.minLow = Math.min(last.minLow, entry.low);
      last.maxHigh = Math.max(last.maxHigh, entry.high);
      last.hasBounded = last.hasBounded || entry.bounded;
      last.rows.push(entry.row);
    } else {
      cliques.push({ maxLow: entry.low, minHigh: entry.high, minLow: entry.low, maxHigh: entry.high, hasBounded: entry.bounded, rows: [entry.row] });
    }
  }
  return cliques.map((clique, index) => {
    const unordered = clique.hasBounded && clique.rows.length > 1;
    if (unordered) clique.rows.sort((a, b) => compareRaidIds(a.id ?? '', b.id ?? ''));
    const label = !unordered ? null
      : Number.isFinite(clique.minLow)
        ? `Unordered · between ${formatDateTime(clique.minLow)} and ${formatDateTime(clique.maxHigh)}`
        : `Unordered · by ${formatDateTime(clique.maxHigh)}`;
    return { key: unordered ? `unordered:${clique.minLow}:${clique.maxHigh}:${index}` : `ordered:${index}`, label, unordered, rows: clique.rows };
  });
}

export function raidLogGroups(data: Pick<RaidLogReply, 'episodes' | 'outcomeNone' | 'outcomeFailure' | 'outcomeCanceled'>, hour: number, types: readonly string[], matches: (tag: string | null, hour: number) => boolean,
  from: number | null): ReadonlyArray<RaidLogGroup> {
  const selected = new Set(types);
  return groupRaidRows(data.episodes.slice(0, startedBy(data.episodes, hour)).flatMap(episode => {
    const at = raidAt(data, episode, hour);
    if (!at || (selected.size && !selected.has(at.episode.raidType ?? '')) || !matches(at.raider, at.since) || (from !== null && at.since < from)) return [];
    return [{ id: episode.id, key: JSON.stringify([episode.id, at.since]), hour: at.since, by: !at.sinceExact, item: at }];
  }));
}

export function raidLogRows(data: Pick<RaidLogReply, 'episodes' | 'outcomeNone' | 'outcomeFailure' | 'outcomeCanceled'>, hour: number, types: readonly string[], matches: (tag: string | null, hour: number) => boolean,
  from: number | null): ReadonlyArray<LogRow<RaidAt>> {
  return raidLogGroups(data, hour, types, matches, from).flatMap(group => group.rows);
}

export function raidLogSections(groups: readonly RaidLogGroup[], months: boolean): ReadonlyArray<LogSection<RaidAt>> | null {
  if (!groups.some(group => group.unordered)) return null;
  const sections: LogSection<RaidAt>[] = [];
  let ordered: LogRow<RaidAt>[] = [];
  const flush = (index: number) => {
    if (!ordered.length) return;
    sections.push({ key: `dated:${index}`, label: 'Dated changes', months, rows: ordered });
    ordered = [];
  };
  groups.forEach((group, index) => {
    if (group.unordered) {
      flush(index);
      sections.push({ key: group.key, label: group.label ?? 'Unordered changes', months: false, rows: group.rows });
    } else {
      ordered.push(...group.rows);
    }
  });
  flush(groups.length);
  return sections;
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
  const role = raidVictimRole(at.victim.source);
  const clause = role === null && at.victim.tag !== null
    ? `${raider} targets ${victim} at ${target}`
    : `${raider} targets ${target} (${role ?? 'target country'}: ${victim})`;
  return `${clause} with ${raidTypeText(data, at.episode)}. ${raidStatus(data, at)}.${preparation ? ` ${preparation}.` : ''}`;
}

export function raidTooltip(data: RaidLogReply, at: RaidAt, raider: string, victim: string, target: string, hour: number): TooltipContent {
  const preparation = raidPreparationText(data, at, hour);
  const end = (at.current ?? at.latest).phase === data.phaseEnded ? at.end : null;
  const endedLabel = end?.source === 'nativeEndDate'
    ? 'Ended (recorded)'
    : end?.exact ? 'Ended (observed)' : 'Ended by (observed)';
  const endedRow = end !== null
    ? [{ label: endedLabel, value: formatDateTime(end.endedHour) }]
    : at.disappeared && at.episode.disappearedHour !== null
      ? [{ label: at.episode.disappearedExact ? 'No longer seen' : 'No longer seen by', value: formatDateTime(at.episode.disappearedHour) }]
      : [];
  return { title: raidTypeText(data, at.episode), icon: raidIcon(data, at.episode) ?? undefined, rows: [
    { label: 'Raider', value: raider }, { label: raidVictimLabel(at.victim.source), value: victim }, { label: 'Target', value: target },
    { label: 'Phase / outcome', value: raidStatus(data, at) },
    ...(preparation !== null ? [{ label: 'Hours prepared', value: preparation === 'Preparation not known' ? 'Not known' : preparation.replace(' h prepared', '') }] : []),
    { label: at.episode.firstHourExact ? 'First seen' : 'First seen by', value: formatDateTime(at.episode.firstHour) },
    ...endedRow,
  ] };
}

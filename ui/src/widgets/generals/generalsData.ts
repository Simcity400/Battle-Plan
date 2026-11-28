import type { GatedTime, PinnedSpec } from '../../dashboard/regionTime';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { liveKey, type LiveValues } from '../../charts/live.ts';
import { createWidget } from '../../dashboard/model.ts';
import type { BoundTrack, WidgetBase } from '../../dashboard/model';
import { subjectToken } from '../../dashboard/chosen.ts';
import type { TraitDate } from './types';
import { unavailableArt } from '../../families/art/types.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import type { LeaderRow as WireRow, LeaderKinds, LeaderPortraitThumbnail, LeaderRole, LeaderTrait, LeaderWithheld, UnitLeadersReply } from '../../generated/dashboard';

const KIND_ARMY = 1;
const KIND_NAVY = 2;

function shaped(input: unknown, keys: readonly string[], what: string): void {
  if (typeof input !== 'object' || input === null || !keys.every(key => key in input)) throw new Error(`The ${what} reply is malformed.`);
}

export function validateLeaders(input: unknown): UnitLeadersReply {
  shaped(input, ['leaders', 'traits', 'withheld', 'missing', 'battlesPartialFrom', 'skillIcons', 'warnings'], 'unit leaders');
  return input as UnitLeadersReply;
}

export interface LeaderRow {
  key: string; hour: number; row: WireRow; name: string; role: LeaderRole; traits: readonly LeaderTrait[];
  fought: number; won: number | null; lost: number | null; unresolved: number; winRate: number | null; foughtPartial: boolean;
  battlesMissing?: string;
}

function battleTotals(fought: number, won: number, lost: number, partial: boolean) {
  const decided = won + lost;
  return { fought, won, lost, unresolved: Math.max(0, fought - decided), winRate: decided ? won * 100 / decided : null, foughtPartial: partial };
}

export const leaderName = (row: { name: string | null }) => row.name ?? 'Character name unavailable';
export function portraitArt(readView: BoundedReadView, portrait: LeaderPortraitThumbnail | null) {
  return portrait?.image
    ? readView.art({ kind: 'url', url: portrait.image })
    : unavailableArt(portrait?.missing ?? 'Portrait unavailable');
}

function recordedBattleTotals(row: WireRow, partial: boolean, live?: LiveValues<number | bigint | string> | null) {
  if (!row.battleOffset) return battleTotals(row.fought, row.won, row.lost, partial);
  const counts = [];
  for (const [at, name] of ['fought', 'won', 'lost'].entries()) {
    const key = liveKey(row.reference, name);
    const value = live?.get(key);
    if (value === undefined) return { ...battleTotals(0, 0, 0, partial), battlesMissing: live?.missing(key) || 'Battle counts not recorded at this hour' };
    counts.push(Math.max(0, Number(value) + row.battleOffset[at]!));
  }
  return battleTotals(counts[0]!, counts[1]!, counts[2]!, partial);
}

function recordedNameOrder(a: string | null, b: string | null): number {
  if (a === b) return 0;
  if (a === null) return 1;
  if (b === null) return -1;
  for (let i = 0; i < Math.min(a.length, b.length);) {
    const left = a.codePointAt(i)!, right = b.codePointAt(i)!;
    if (left !== right) return left - right;
    i += left > 0xffff ? 2 : 1;
  }
  return a.length - b.length;
}

const score = (row: LeaderRow) => row.row.components.reduce((total, n) => total + n, 0);

function rankLeaders(a: LeaderRow, b: LeaderRow) {
  return (b.row.skill ?? -Infinity) - (a.row.skill ?? -Infinity)
    || Number(Boolean(a.battlesMissing)) - Number(Boolean(b.battlesMissing))
    || (b.won ?? 0) - (a.won ?? 0) || b.fought - a.fought
    || score(b) - score(a) || recordedNameOrder(a.row.name, b.row.name)
    || a.key.length - b.key.length || (a.key < b.key ? -1 : Number(a.key > b.key));
}

export function leaderRows(reply: UnitLeadersReply, hour: number, countries: ReadonlySet<string> | null, kind: LeaderKinds, live?: LiveValues<number | bigint | string> | null): LeaderRow[] {
  const partial = reply.battlesPartialFrom !== null && reply.battlesPartialFrom <= hour;
  const rows = reply.leaders.flatMap(row => {
    if (row.role.kind !== (kind === 'navy' ? KIND_NAVY : KIND_ARMY)) return [];
    if (countries && !(row.role.owner !== null && countries.has(row.role.owner))) return [];
    return [{ key: row.reference, hour, row, name: leaderName(row), role: row.role, traits: (row.traits ?? []).map(at => reply.traits[at]!),
      ...recordedBattleTotals(row, partial, live) }];
  });
  if (reply.leaders.some(row => row.battleOffset)) rows.sort(rankLeaders);
  return rows;
}

export function withheldLeaders(reply: UnitLeadersReply, countries: ReadonlySet<string> | null, kind: LeaderKinds): LeaderWithheld[] {
  return reply.withheld.filter(leader => leader.kind === (kind === 'navy' ? KIND_NAVY : KIND_ARMY)
    && (leader.owner === null || countries === null || countries.has(leader.owner)));
}

export function withheldLeaderState(leaders: readonly LeaderWithheld[], country?: string) {
  const count = new Set(leaders.map(leader => leader.reference)).size;
  const owners = country ? `of ${country}’s` : 'of these countries’';
  return {
    message: `${count} ${owners} ${count === 1 ? 'leader' : 'leaders'} can’t be shown: ${[...new Set(leaders.map(leader => leader.reason))].join('; ')}`,
    detail: leaders.map(leader => `${leader.name ?? `Leader ${leader.reference}`}: ${leader.reason}`).join('\n'),
  };
}

export function traitDateText(date: TraitDate | null): string {
  return date ? `${date[1] === 0 ? 'Held before' : 'Gained'} ${formatDateTime(date[0])}` : 'Date not recorded';
}

export function pinnedCareer(source: WidgetBase, reference: string, track: Pick<BoundTrack, 'id' | 'branch' | 'source'>, time: GatedTime): PinnedSpec {
  const base = createWidget('general-career', track.id);
  return { ...base, campaign: source.campaign, trackIds: [...source.trackIds], countries: source.countries, time,
    contentScale: source.contentScale, options: { leader: subjectToken(track.source.metadata.campaignKey, reference) } };
}

import type { GatedTime, PinnedSpec } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { createWidget } from '../../dashboard/model.ts';
import type { BoundTrack, WidgetBase } from '../../dashboard/model';
import { subjectToken } from '../../dashboard/chosen.ts';
import type { TraitDate } from './types';
import type { LeaderRow as WireRow, LeaderKinds, LeaderRole, LeaderSelection, LeaderTrait, LeaderWithheld, UnitLeadersReply } from '../../generated/dashboard';

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
}

function battleTotals(fought: number, won: number, lost: number, partial: boolean) {
  const decided = won + lost;
  return { fought, won, lost, unresolved: Math.max(0, fought - decided), winRate: decided ? won * 100 / decided : null, foughtPartial: partial };
}

export const leaderName = (row: { name: string | null }) => row.name ?? 'Character name unavailable';
export const attributeNames = (role: Pick<LeaderRole, 'attributeNames'>) => role.attributeNames.map(name => name ?? 'Name not recorded');

export function leaderRows(reply: UnitLeadersReply, hour: number, countries: ReadonlySet<string> | null, kind: LeaderKinds): LeaderRow[] {
  const partial = reply.battlesPartialFrom !== null && reply.battlesPartialFrom <= hour;
  return reply.leaders.flatMap(row => {
    if (row.role.kind !== (kind === 'navy' ? KIND_NAVY : KIND_ARMY)) return [];
    if (countries && !(row.role.owner !== null && countries.has(row.role.owner))) return [];
    return [{ key: row.reference, hour, row, name: leaderName(row), role: row.role, traits: (row.traits ?? []).map(at => reply.traits[at]!),
      ...battleTotals(row.fought, row.won, row.lost, partial) }];
  });
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

export function leaderRequestSelection(tags: readonly string[] | null): LeaderSelection {
  return tags === null ? { kind: 'all' } : { kind: 'countries', countries: tags.map(countryFromKey) };
}

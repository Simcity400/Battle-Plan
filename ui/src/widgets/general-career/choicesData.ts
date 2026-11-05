import { leaderName } from '../generals/generalsData.ts';
import type { LeaderChoice, LeaderChoicesReply } from '../../generated/dashboard';
import { subjectToken } from '../../dashboard/chosen.ts';
import { pagePreparation, registerPageJoin } from '../../dashboard/pageJoins.ts';
import { finalPreparedReply } from '../../dashboard/preparedPrefix.ts';
import { object, opaqueTag, signed32, unsigned64 } from '../../dashboard/validation.ts';

/** Joined pages keep the last one's `cursor`: a string while pages remain to be read. */
export function joinLeaderChoicePages(pages: readonly unknown[]): unknown {
  const replies = pages as readonly LeaderChoicesReply[];
  const last = replies[replies.length - 1];
  return { ...replies[0], ...pagePreparation(last), cursor: last.cursor, choices: replies.flatMap(reply => reply.choices) };
}
registerPageJoin(['unit-leaders-choices'], joinLeaderChoicePages);

const isChoice = (value: unknown): value is LeaderChoice => Array.isArray(value) && value.length === 4 && unsigned64(value[0])
  && (value[1] === null || typeof value[1] === 'string') && (value[2] === null || opaqueTag(value[2])) && signed32(value[3]);

/** The picker's leaders in reference order, one page or the pages joined so far; `cursor` names
 * the page after them. */
export function validateLeaderChoicesReply(input: unknown): LeaderChoicesReply {
  const bad = (): never => { throw new Error('The leader choices reply is malformed.'); };
  if (!object(input) || !unsigned64(input.branchId) || !(input.cursor === null || unsigned64(input.cursor))) return bad();
  const listed: unknown = input.choices;
  if (!Array.isArray(listed) || !listed.every(isChoice)) return bad();
  const rows = listed as LeaderChoice[];
  // Across joined pages too: a page boundary that moved would repeat a leader.
  for (let at = 1; at < rows.length; at++) if (BigInt(rows[at - 1][0]) >= BigInt(rows[at][0])) return bad();
  return input as unknown as LeaderChoicesReply;
}

/** Every leader, once each page is read and preparation is complete; none before. */
export const finalLeaderChoices = (reply: LeaderChoicesReply | undefined): readonly LeaderChoice[] | undefined =>
  reply && reply.cursor === null && finalPreparedReply(reply) ? reply.choices : undefined;

export function leaderChoices(choices: readonly LeaderChoice[], campaignKey: string, countryName: (hour: number, tag: string) => { name: string; nameDetail?: string }) {
  return choices.map(([reference, recorded, owner, lastHour]) => {
    const name = leaderName({ person: { name: recorded } });
    const country = owner === null ? undefined : countryName(lastHour, owner);
    return { ...(country?.nameDetail ? { detail: country.nameDetail } : {}), value: subjectToken(campaignKey, reference), name, label: `${name} · ${country?.name ?? 'Owner not recorded'}` };
  }).sort((a, b) => a.name.localeCompare(b.name) || a.value.localeCompare(b.value)).map(({ name: _name, ...choice }) => choice);
}

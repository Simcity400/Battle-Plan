import type { ShareRun } from '../../charts/share';
import type { HolderAt } from '../../dashboard/holderLooks';
import type { FactionAccountDetail, FactionDetailReply, FactionInfluenceAccount, FactionInfluenceReply } from '../../generated/dashboard';

/** Influence shares are recorded as percentages with three decimals: the whole is 100000. */
export const SHARE_SCALE = 3;
export const WHOLE_SHARE = 100000;

type InfluenceAccount = FactionInfluenceAccount;
type Span = { firstHour: number; lastHour: number };
const covers = (span: Span, hour: number) => span.firstHour <= hour && hour <= span.lastHour;
const runAt = <T extends Span>(runs: readonly T[], hour: number) => runs.find(run => covers(run, hour)) ?? null;

/** The accounts that were members at some hour, in reply order: one band each. */
export const memberAccounts = (reply: FactionInfluenceReply) => reply.accounts.filter(account => account.member.length > 0);

/** Each account as the native holder it is, named and coloured at the last hour it was a
 * member: a tag a later country takes never names or colours an earlier account. */
export const accountHolders = (accounts: readonly InfluenceAccount[]): HolderAt[] =>
  accounts.map(account => ({ key: account.key, tag: account.tag, hour: account.member.at(-1)!.lastHour }));

/** Stacked influence shares over time: a column per stretch where the members and every
 * member's recorded share are constant, as the recorded integers out of the whole, with a
 * last band for the share no current member holds, so a lone 25% stays 25%. A member whose
 * share is not known leaves the whole stretch a gap; a non-member contributes nothing. */
export function influenceShareRuns(accounts: readonly InfluenceAccount[]): ShareRun[] {
  const cuts = new Set<number>();
  for (const account of accounts) {
    for (const run of account.runs) { cuts.add(run.firstHour); cuts.add(run.lastHour + 1); }
    for (const span of account.member) { cuts.add(span.firstHour); cuts.add(span.lastHour + 1); }
  }
  const hours = [...cuts].sort((a, b) => a - b);
  const result: ShareRun[] = [];
  for (let index = 0; index + 1 < hours.length; index++) {
    const firstHour = hours[index], lastHour = hours[index + 1] - 1;
    let members = 0, known = true, starts = false;
    const counts = accounts.map(account => {
      if (!account.member.some(span => covers(span, firstHour))) return 0;
      members++;
      const run = runAt(account.runs, firstHour);
      if (!run || run.shareRaw === null) { known = false; return 0; }
      if (run.firstHour === firstHour && run.startsSegment) starts = true;
      return Math.max(0, Number(run.shareRaw));
    });
    if (!members || !known) continue;
    counts.push(Math.max(0, WHOLE_SHARE - counts.reduce((sum, count) => sum + count, 0)));
    const previous = result.at(-1);
    result.push({ firstHour, lastHour, counts, ...(starts || !previous || previous.lastHour + 1 !== firstHour ? { startsSegment: true } : {}) });
  }
  return result;
}

/** One member's share of the faction against the rest, for its drill. */
export function memberShareRuns(account: InfluenceAccount): ShareRun[] {
  return account.runs.flatMap(run => run.shareRaw === null ? [] : account.member.flatMap(span => {
    const firstHour = Math.max(run.firstHour, span.firstHour), lastHour = Math.min(run.lastHour, span.lastHour);
    if (firstHour > lastHour) return [];
    const share = Math.max(0, Math.min(WHOLE_SHARE, Number(run.shareRaw)));
    return [{ firstHour, lastHour, counts: [share, WHOLE_SHARE - share], ...(run.startsSegment ? { startsSegment: true } : {}) }];
  }));
}

/** The members ranked at the hour shown: current members only (retained accounts of departed
 * countries are not members); influence share is the recorded share, never re-normalised. */
export function rankingRows(data: FactionDetailReply): FactionAccountDetail[] {
  return (data.accounts ?? []).filter(account => account.currentMember);
}

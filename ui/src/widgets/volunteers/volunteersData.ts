import type { FlowPart } from '../../charts/flow/sum';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { MilitaryAidKind, MilitaryAidLink, MilitaryAidRun, VolunteersExpeditionariesReply } from '../../generated/dashboard';

type VolunteerRun = MilitaryAidRun;
type VolunteerLink = MilitaryAidLink;
type VolunteersReply = VolunteersExpeditionariesReply;

export type VolunteerKind = MilitaryAidKind;
/** A link in place at an hour. `senderKey` and `hostKey` name its native holders (`holderKey`). */
export interface LinkAt { key: string; sender: string; host: string; senderKey: string; hostKey: string; divisions: number; since: number }

/** A country holder as the diagram keys it: its tag with its slot and country id, so two
 * countries holding one tag in turn are two nodes, and a holder not known is its own. */
export const holderKey = (tag: string, slot: number | null, countryId: number | null) => JSON.stringify([tag, slot, countryId]);

/** Which links count at an hour. */
export type LinkFilter = (link: Pick<VolunteerLink, 'sender' | 'host'>, hour: number) => boolean;

const runAt = (runs: readonly VolunteerRun[], hour: number) => runs.find(run => run.firstHour <= hour && hour <= run.lastHour) ?? null;

/** The links in place at `hour` with a known positive count, and how many links are unknown
 * then (counted, never drawn as zero). `since` is where the link's current stretch began. */
export function linksAt(reply: VolunteersReply, hour: number, keep: LinkFilter): { links: LinkAt[]; unknown: number } {
  const links: LinkAt[] = [];
  let unknown = 0;
  for (const link of reply.links) {
    if (!keep(link, hour)) continue;
    const run = runAt(link.runs, hour);
    if (!run) continue;
    if (run.divisions === null) { unknown++; continue; }
    if (run.divisions <= 0) continue;
    let index = link.runs.indexOf(run);
    while (index > 0 && !link.runs[index].startsSegment && link.runs[index - 1].lastHour + 1 === link.runs[index].firstHour && (link.runs[index - 1].divisions ?? 0) > 0) index--;
    links.push({ key: link.key, sender: link.sender, host: link.host, senderKey: holderKey(link.sender, link.senderSlot, link.senderCountryId),
      hostKey: holderKey(link.host, link.hostSlot, link.hostCountryId), divisions: run.divisions, since: link.runs[index].firstHour });
  }
  return { links, unknown };
}

/** Countries chosen: their links with anyone, or only links between them. `matches` is the
 * shared country choice (`countryMatch`): it passes a tag at an hour by the native holder
 * carrying it then, under the group's membership setting; null passes every country. */
export function linkFilter(matches: ((tag: string, hour: number) => boolean) | null, between: boolean): LinkFilter {
  if (!matches) return () => true;
  return (link, hour) => between ? matches(link.sender, hour) && matches(link.host, hour) : matches(link.sender, hour) || matches(link.host, hour);
}

/** The tags a request names: every tag the chosen holders carry in the branch
 * (`countryMatchFilter`'s alias runs), so the reply holds only links that can touch them;
 * null, every link, only for All countries. */
export const requestTags = (all: boolean, aliases: readonly { tag: string }[] | null): string[] | null =>
  all ? null : [...new Set((aliases ?? []).map(run => run.tag))].sort(compareTags);

const PREPARING = 'This range is still being prepared.';

/** What the widget shows when the countries chosen name no tag to request, so nothing is
 * read (never every link): groups still being prepared are loading; a group counted at
 * each hour whose membership was not recorded at the hour shown is unavailable there,
 * and one recorded with no members is an empty group; a choice of countries none of
 * whose holders the campaign has is not in it. */
export function withoutRequest(state: { settled: boolean; hourly: boolean; membershipUnobserved: boolean }):
  { kind: 'loading' | 'unavailable' | 'empty'; message: string } {
  if (!state.settled) return { kind: 'loading', message: PREPARING };
  if (state.hourly) return state.membershipUnobserved ? { kind: 'unavailable', message: 'This group’s membership was not recorded at this date.' }
    : { kind: 'empty', message: 'This group has no members at this date.' };
  return { kind: 'empty', message: 'The countries chosen are not in this campaign.' };
}

/** What the diagram says when no link shows at an hour: unrecorded aid or group membership
 * is not an empty roster, and a link whose count is unknown is not none. */
export function emptyMessage(kind: VolunteerKind, state: { aidUnobserved: boolean; membershipUnobserved: boolean; membershipPending?: boolean; unknown: number }): string {
  if (state.aidUnobserved) return 'Not recorded at this date.';
  if (state.membershipPending) return PREPARING;
  if (state.membershipUnobserved) return 'This group’s membership was not recorded at this date.';
  if (state.unknown) return 'These forces are not known at this date.';
  return kind === 'land' ? 'No volunteers abroad at this date.' : 'No expeditionary forces lent at this date.';
}

/** The links as flow parts between native holders, so a holder never stands in for another. */
export const flowParts = (links: readonly LinkAt[]): FlowPart<LinkAt>[] => links.map(link => ({ part: link, source: link.senderKey, target: link.hostKey, value: link.divisions }));

/** The largest side total any hour reaches, so one scale holds all campaign. Totals change
 * only where a link's count does or a country joins the countries chosen (`boundaries`). */
export function campaignScale(reply: VolunteersReply, keep: LinkFilter, boundaries: Iterable<number> = []): number {
  const hours = new Set<number>(boundaries);
  for (const link of reply.links) for (const run of link.runs) hours.add(run.firstHour);
  let largest = 0;
  for (const hour of hours) {
    const { links } = linksAt(reply, hour, keep);
    largest = Math.max(largest, links.reduce((sum, link) => sum + link.divisions, 0));
  }
  return largest;
}

/** One holder's partners at the hour: what it sends to each host and receives from each sender. */
export function partnersOf(links: readonly LinkAt[], holder: string): { sent: LinkAt[]; received: LinkAt[] } {
  return { sent: links.filter(link => link.senderKey === holder).sort((a, b) => b.divisions - a.divisions),
    received: links.filter(link => link.hostKey === holder).sort((a, b) => b.divisions - a.divisions) };
}

/** Each holder in `links` with the tag it holds and the earliest hour its links show it
 * holding that tag: a stable hour to read its look at as playback moves. */
export function linkHolders(links: readonly LinkAt[]): { key: string; tag: string; hour: number }[] {
  const holders = new Map<string, { key: string; tag: string; hour: number }>();
  const add = (key: string, tag: string, hour: number) => { const known = holders.get(key); if (!known || hour < known.hour) holders.set(key, { key, tag, hour }); };
  for (const link of links) { add(link.senderKey, link.sender, link.since); add(link.hostKey, link.host, link.since); }
  return [...holders.values()];
}

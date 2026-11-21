import type { FlowPart } from '../../charts/flow/sum';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { MilitaryAidKind, MilitaryAidLink, MilitaryAidRun, VolunteersExpeditionariesReply } from '../../generated/dashboard';

type VolunteerRun = MilitaryAidRun;
type VolunteerLink = MilitaryAidLink;
type VolunteersReply = VolunteersExpeditionariesReply;

export type VolunteerKind = MilitaryAidKind;
export interface LinkAt { key: string; sender: string; host: string; senderKey: string; hostKey: string; divisions: number; since: number }

export const holderKey = (tag: string, slot: number | null, countryId: number | null) => JSON.stringify([tag, slot, countryId]);

export type LinkFilter = (link: Pick<VolunteerLink, 'sender' | 'host'>, hour: number) => boolean;

const runAt = (runs: readonly VolunteerRun[], hour: number) => runs.find(run => run.firstHour <= hour && hour <= run.lastHour) ?? null;

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

export function linkFilter(matches: ((tag: string, hour: number) => boolean) | null, between: boolean): LinkFilter {
  if (!matches) return () => true;
  return (link, hour) => between ? matches(link.sender, hour) && matches(link.host, hour) : matches(link.sender, hour) || matches(link.host, hour);
}

export const requestTags = (all: boolean, aliases: readonly { tag: string }[] | null): string[] | null =>
  all ? null : [...new Set((aliases ?? []).map(run => run.tag))].sort(compareTags);

export function withoutRequest(state: { hourly: boolean; membershipUnobserved: boolean }):
  { kind: 'unavailable' | 'empty'; message: string } {
  if (state.hourly) return state.membershipUnobserved ? { kind: 'unavailable', message: 'This group’s membership was not recorded at this date.' }
    : { kind: 'empty', message: 'This group has no members at this date.' };
  return { kind: 'empty', message: 'The countries chosen are not in this campaign.' };
}

export function emptyMessage(kind: VolunteerKind, state: { aidUnobserved: boolean; membershipUnobserved: boolean; unknown: number }): string {
  if (state.aidUnobserved) return 'Not recorded at this date.';
  if (state.membershipUnobserved) return 'This group’s membership was not recorded at this date.';
  if (state.unknown) return 'These forces are not known at this date.';
  return kind === 'land' ? 'No volunteers abroad at this date.' : 'No expeditionary forces lent at this date.';
}

export const flowParts = (links: readonly LinkAt[]): FlowPart<LinkAt>[] => links.map(link => ({ part: link, source: link.senderKey, target: link.hostKey, value: link.divisions }));

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

export function partnersOf(links: readonly LinkAt[], holder: string): { sent: LinkAt[]; received: LinkAt[] } {
  return { sent: links.filter(link => link.senderKey === holder).sort((a, b) => b.divisions - a.divisions),
    received: links.filter(link => link.hostKey === holder).sort((a, b) => b.divisions - a.divisions) };
}

export function linkHolders(links: readonly LinkAt[]): { key: string; tag: string; hour: number }[] {
  const holders = new Map<string, { key: string; tag: string; hour: number }>();
  const add = (key: string, tag: string, hour: number) => { const known = holders.get(key); if (!known || hour < known.hour) holders.set(key, { key, tag, hour }); };
  for (const link of links) { add(link.senderKey, link.sender, link.since); add(link.hostKey, link.host, link.since); }
  return [...holders.values()];
}

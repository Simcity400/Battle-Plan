import type { CountryFlagLook } from '../../components/CountryFlag';
import type { BandStep, TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { bridgeGaps, coverCounts, describeRun, runState, stretchesOf, withoutHours } from '../../charts/timeline/runState.ts';
import type { HourRange, RunWords } from '../../charts/timeline/runState';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity';
import type { FactionName } from '../../dashboard/factionNames';
import { UNNAMED_FACTION } from '../../dashboard/factionNames.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { RankedCounts } from '../../dashboard/groupCounts.ts';
import type { CountryGroupReply, FactionGroup, HourSpan, MembershipRun } from '../../generated/dashboard';


export type FactionOrder = 'founded' | 'members' | 'name';
export type MemberOrder = 'joined' | 'name';

export const LEADER_GLYPH = '★';

export interface MemberView {
  key: string;
  runs: MembershipRun[];
  stretches: HourRange[];
}

export interface FactionView {
  id: string;
  countryIds: ReadonlyMap<string, number>;
  name: string | null;
  nameAt: (hour: number) => FactionName;
  color: string;
  existence: HourRange[];
  lifetime: HourRange[];
  members: MemberView[];
  identities: CountryIdentityIndex;
  leaders: MembershipRun[];
  counts: BandStep[];
  unknown: HourSpan[];
}

function merged(spans: readonly HourRange[]): HourSpan[] {
  const sorted = [...spans].sort((a, b) => a.firstHour - b.firstHour);
  const result: HourSpan[] = [];
  for (const span of sorted) {
    const last = result.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else result.push({ firstHour: span.firstHour, lastHour: span.lastHour });
  }
  return result;
}

export function factionGaps(reply: CountryGroupReply): HourSpan[] {
  return reply.unobserved.factions;
}

export function factionViews(reply: CountryGroupReply, name: (faction: FactionGroup, hour?: number) => FactionName, colour: (id: string) => string): FactionView[] {
  const identities = countryIdentityIndex(reply.countryIdentities);
  const gaps = factionGaps(reply);
  const allUnobserved = merged(gaps);
  return reply.factions.map(faction => {
    const byHolder = new Map<string, MembershipRun[]>();
    const missing: HourRange[] = [];
    for (const member of faction.members) {
      const known = identities.membershipRuns([member]);
      for (const span of withoutHours([member], known)) missing.push(span);
      for (const run of known) {
        const key = identities.keyAt(run.tag, run.firstHour)!;
        const runs = byHolder.get(key);
        if (runs) runs.push(run); else byHolder.set(key, [run]);
      }
    }
    const unknown = merged([...gaps, ...faction.incomplete, ...missing]);
    const members = [...byHolder].map(([key, runs]) => {
      runs.sort((a, b) => a.firstHour - b.firstHour);
      return { key, runs, stretches: bridgeGaps(runs, unknown).map(({ firstHour, lastHour }) => ({ firstHour, lastHour })) };
    }).sort((a, b) => a.runs[0].firstHour - b.runs[0].firstHour);
    const existence = withoutHours([{ firstHour: faction.firstHour, lastHour: faction.lastHour }], allUnobserved);
    return {
      id: faction.id, countryIds: identities.firstHoursDuring(faction.members), name: name(faction).name, color: colour(faction.id),
      nameAt: hour => name(faction, hour),
      existence, lifetime: bridgeGaps(existence, allUnobserved).map(({ firstHour, lastHour }) => ({ firstHour, lastHour })),
      members, identities, leaders: identities.membershipRuns(faction.leaders).sort((a, b) => a.firstHour - b.firstHour),
      counts: withoutHours(coverCounts(faction.members), unknown), unknown,
    };
  });
}

export function tagAt(view: FactionView, member: MemberView, hour: number): string | null {
  return view.identities.aliasesAt(new Set([member.key]), hour)[0] ?? null;
}

export function leaderAt(leaders: readonly MembershipRun[], hour: number): string | null {
  return leaders.find(run => run.firstHour <= hour && hour <= run.lastHour)?.tag ?? null;
}

export function membersAt(view: FactionView, hour: number): number | null {
  if (view.unknown.some(span => span.firstHour <= hour && hour <= span.lastHour)) return null;
  return view.counts.find(step => step.firstHour <= hour && hour <= step.lastHour)?.value ?? 0;
}

export function leadershipParts(run: MembershipRun, leaders: readonly MembershipRun[]): { firstHour: number; lastHour: number; leader: boolean }[] {
  const led = merged(leaders.filter(item => item.tag === run.tag && item.firstHour <= run.lastHour && run.firstHour <= item.lastHour)
    .map(item => ({ firstHour: Math.max(item.firstHour, run.firstHour), lastHour: Math.min(item.lastHour, run.lastHour) })));
  const parts: { firstHour: number; lastHour: number; leader: boolean }[] = [];
  let from = run.firstHour;
  for (const span of led) {
    if (span.firstHour > from) parts.push({ firstHour: from, lastHour: span.firstHour - 1, leader: false });
    parts.push({ ...span, leader: true });
    from = span.lastHour + 1;
  }
  if (from <= run.lastHour) parts.push({ firstHour: from, lastHour: run.lastHour, leader: false });
  return parts;
}

export interface FactionText {
  country: (tag: string) => CountryFlagLook & { name: string };
  day: (hour: number) => string;
}

export interface LaneOptions {
  order: FactionOrder;
  members: MemberOrder;
  ended: boolean;
  countries: ReadonlySet<string> | null;
  countryIds?: ReadonlySet<string> | null;
  counts: RankedCounts | null;
}

export const UNNAMED = UNNAMED_FACTION;

export function factionNameShown(view: FactionView, hour: number): FactionName {
  const end = view.lifetime.at(-1)?.lastHour;
  return view.nameAt(end !== undefined && hour > end ? end : hour);
}

export const factionLabel = (view: FactionView, hour?: number) => (hour == null ? view.name : factionNameShown(view, hour).name) ?? UNNAMED;

function memberWords(day: (hour: number) => string): RunWords {
  return {
    since: hour => `Joined ${day(hour)}`, before: hour => `Member before ${day(hour)}`,
    ended: hour => `Left ${day(hour)}`, lastSeen: hour => `Last recorded ${day(hour)}`,
  };
}

export function factionWords(day: (hour: number) => string): RunWords {
  return {
    since: hour => `Founded ${day(hour)}`, before: hour => `Founded before ${day(hour)}`,
    ended: hour => `Dissolved ${day(hour)}`, lastSeen: hour => `Last recorded ${day(hour)}`,
  };
}

const firstHour = (runs: readonly HourRange[]) => runs[0]?.firstHour ?? Infinity;

export interface LaneInfo { faction: FactionView; member: MemberView | null }

export const laneId = (faction: string, member: string | null) => JSON.stringify(member == null ? ['faction', faction] : ['member', faction, member]);

function heldThrough(runs: readonly HourRange[], gaps: readonly HourRange[], hour: number): HourRange[] {
  const known = runs.filter(run => run.firstHour <= hour).map(run => ({ firstHour: run.firstHour, lastHour: Math.min(run.lastHour, hour) }));
  const seen = gaps.filter(gap => gap.firstHour <= hour).map(gap => ({ firstHour: gap.firstHour, lastHour: Math.min(gap.lastHour, hour) }));
  return bridgeGaps(known, seen).map(({ firstHour, lastHour }) => ({ firstHour, lastHour }));
}

export function factionLanes(views: readonly FactionView[], hour: number, options: LaneOptions, text: FactionText,
  unobserved: readonly HourSpan[], branchFirstHour: number): { lanes: TimelineLane[]; info: Map<string, LaneInfo> } {
  const lanes: TimelineLane[] = [];
  const lifetimes = new Map(views.map(view => [view, heldThrough(view.existence, unobserved, hour)]));
  const lifetime = (view: FactionView) => lifetimes.get(view)!;
  const info = new Map<string, LaneInfo>();
  const chosen = options.countryIds ?? (options.countries === null ? null : new Set<string>());
  const involves = (view: FactionView) => { for (const [key, since] of view.countryIds) if (since <= hour && chosen!.has(key)) return true; return false; };
  const shown = views.filter(view => firstHour(view.existence) <= hour
    && (options.ended || runState(lifetime(view), hour, unobserved).kind !== 'ended')
    && (!chosen || involves(view)));
  const founded = (a: FactionView, b: FactionView) => firstHour(a.existence) - firstHour(b.existence) || compareIds(a.id, b.id);
  const rank = (view: FactionView) => options.counts?.rank.get(view.id) ?? Infinity;
  const names = options.order === 'name' ? new Map(shown.map(view => [view, factionLabel(view, hour)])) : null;
  shown.sort(options.order === 'name' ? (a, b) => names!.get(a)!.localeCompare(names!.get(b)!) || compareIds(a.id, b.id)
    : options.order === 'members' ? (a, b) => rank(a) - rank(b) || founded(a, b)
      : founded);
  for (const view of shown) {
    const name = names?.get(view) ?? factionLabel(view, hour);
    const exists = runState(lifetime(view), hour, unobserved).kind === 'held';
    const leader = leaderAt(view.leaders, hour);
    const members = options.counts?.count(view.id)?.count ?? null;
    const state = exists
      ? members == null ? 'Members not recorded' : `${members} ${members === 1 ? 'member' : 'members'}`
      : describeRun(lifetime(view), hour, factionWords(text.day), unobserved, branchFirstHour) ?? '';
    const headingId = laneId(view.id, null);
    info.set(headingId, { faction: view, member: null });
    lanes.push({
      id: headingId, heading: true, opens: true, label: state, identity: { label: name }, icons: false, names: false,
      muted: !exists,
      spans: stretchesOf(view.counts).map(stretch => ({
        id: `${stretch.firstHour}`, firstHour: stretch.firstHour, lastHour: stretch.lastHour,
        label: name, color: view.color, band: stretch.steps,
      })),
    });
    const memberUnobserved = view.unknown;
    const stretches = new Map(view.members.map(member => [member, heldThrough(member.runs, memberUnobserved, hour)]));
    const shownMembers = view.members.filter(member => firstHour(member.runs) <= hour
      && (options.ended || runState(stretches.get(member)!, hour, memberUnobserved).kind !== 'ended'));
    const tags = new Map(shownMembers.map(member => [member, tagAt(view, member, hour)]));
    const tagOf = (member: MemberView) => tags.get(member) ?? null;
    const leading = (member: MemberView) => leader !== null && leader === tagOf(member) ? 0 : 1;
    const countryAt = (tag: string | null) => tag === null ? { name: 'Unknown country', flagReason: 'Unknown country' } : text.country(tag);
    const nameOf = (member: MemberView) => countryAt(tagOf(member)).name;
    shownMembers.sort(options.members === 'name' ? (a, b) => nameOf(a).localeCompare(nameOf(b)) || compareTags(a.key, b.key)
      : (a, b) => firstHour(a.runs) - firstHour(b.runs) || leading(a) - leading(b) || nameOf(a).localeCompare(nameOf(b)) || compareTags(a.key, b.key));
    for (const member of shownMembers) {
      const tag = tagOf(member);
      const country = countryAt(tag);
      const now = describeRun(stretches.get(member)!, hour, memberWords(text.day), memberUnobserved, branchFirstHour) ?? '';
      const ended = runState(stretches.get(member)!, hour, memberUnobserved).kind === 'ended';
      const spans: TimelineSpan[] = member.runs.flatMap(run => leadershipParts(run, view.leaders).map(part => ({
        id: `${part.firstHour}`, firstHour: part.firstHour, lastHour: part.lastHour, label: country.name,
        color: view.color, ...(part.leader ? { glyph: LEADER_GLYPH } : {}),
      })));
      const id = laneId(view.id, member.key);
      info.set(id, { faction: view, member });
      lanes.push({
        id, depth: 1, identity: { label: country.name, countryFlag: country }, icons: false, names: false, muted: ended,
        label: leader !== null && leader === tag ? `${LEADER_GLYPH} Leader · ${now}` : now, spans,
      });
    }
  }
  return { lanes, info };
}

function compareIds(a: string, b: string): number {
  return a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);
}

export function laneCountries(lanes: readonly TimelineLane[], info: ReadonlyMap<string, LaneInfo>, hour: number): string[] {
  const tags = new Set<string>();
  for (const lane of lanes) {
    const about = info.get(lane.id);
    if (!about) continue;
    const tag = about.member ? tagAt(about.faction, about.member, hour) : leaderAt(about.faction.leaders, hour);
    if (tag != null) tags.add(tag);
  }
  return [...tags];
}

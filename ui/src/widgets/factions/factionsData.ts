import type { BandStep, TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { bridgeGaps, coverCounts, describeRun, runState, stretchesOf, withoutHours } from '../../charts/timeline/runState.ts';
import type { HourRange, RunWords } from '../../charts/timeline/runState';
import { Lineage, joinTagChanges } from '../../dashboard/identity.ts';
import type { FactionName } from '../../dashboard/factionNames';
import { UNNAMED_FACTION } from '../../dashboard/factionNames.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryGroupReply, FactionGroup, HourSpan, MembershipRun } from '../../generated/dashboard';

/** The Factions widget's data: every recorded faction (any number, including ones
 * founded or dissolved partway through) from the `country-groups` history, as a
 * band per faction whose thickness is its member count, and a lane per member from
 * join to leave, its leader marked. Nothing here knows a vanilla faction: names,
 * members and leaders all come from the recording. */

export type FactionOrder = 'founded' | 'members' | 'name';
export type MemberOrder = 'joined' | 'name';

/** The mark on a leader's stretch of a member bar. */
export const LEADER_GLYPH = '★';

/** One member country of a faction: its runs across any change of tag, in hour order. */
export interface MemberView {
  /** Unique within its faction: its country (`Lineage.countryAt`). */
  key: string;
  runs: MembershipRun[];
  /** Its membership where nothing recorded says it stopped (`bridgeGaps`): runs joined
   * across unrecorded hours and changes of tag. What its state is said of. */
  stretches: HourRange[];
}

export interface FactionView {
  id: string;
  /** Latest summary, used only when no hour is requested. */
  name: string | null;
  /** Dated name lookups, sharing the prepared history and campaign override. */
  nameAt: (hour: number) => FactionName;
  color: string;
  /** Recorded hours the faction existed (its first to last hour, less unrecorded ones). */
  existence: HourRange[];
  /** Its existence joined across unrecorded hours: what its state is said of. */
  lifetime: HourRange[];
  /** Each member country, joined across its changes of tag, by first membership. */
  members: MemberView[];
  /** The countries behind the tags (`Lineage`), shared by every faction of a reply. */
  lineage: Lineage;
  /** Who led it (the member at order zero), in hour order. */
  leaders: MembershipRun[];
  /** How many members it had over its known hours: its band's steps. */
  counts: BandStep[];
  /** Hours its roster is not known: none recorded, or one naming an unknown country. */
  unknown: HourSpan[];
}

/** Sorted, merged spans. */
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

/** Each faction of the reply, ready to draw: its name (`name`), colour (`colour`),
 * existence, members, leaders and member counts. A member whose tag changed (the
 * reply's `identityChanges`) is one member, not one leaving and another joining. */
export function factionViews(reply: CountryGroupReply, name: (faction: FactionGroup, hour?: number) => FactionName, colour: (id: string) => string): FactionView[] {
  const lineage = new Lineage(reply.identityChanges);
  const allUnobserved = merged(reply.unobserved.factions);
  return reply.factions.map(faction => {
    const unknown = merged([...reply.unobserved.factions, ...faction.incomplete]);
    const members = joinTagChanges(faction.members, lineage).map(runs => ({
      // Unique however tags are reused: the member's country (`Lineage`).
      key: lineage.countryAt(runs[0].tag, runs[0].firstHour), runs, stretches: bridgeGaps(runs, unknown).map(({ firstHour, lastHour }) => ({ firstHour, lastHour })),
    }));
    const existence = withoutHours([{ firstHour: faction.firstHour, lastHour: faction.lastHour }], allUnobserved);
    return {
      id: faction.id, name: name(faction).name, color: colour(faction.id),
      nameAt: hour => name(faction, hour),
      existence, lifetime: bridgeGaps(existence, allUnobserved).map(({ firstHour, lastHour }) => ({ firstHour, lastHour })),
      members, lineage, leaders: [...faction.leaders].sort((a, b) => a.firstHour - b.firstHour),
      counts: withoutHours(coverCounts(faction.members), unknown), unknown,
    };
  });
}

/** The tag a member's country had at an hour, through its whole history of tags
 * (`Lineage.tagOf`), so a former member renamed after it left is named by its tag then,
 * never by one another country took later. */
export function tagAt(view: FactionView, member: MemberView, hour: number): string {
  return view.lineage.tagOf(member.key, hour);
}

/** The members of a faction at an hour, by their tag then. */
export function membersHeld(view: FactionView, hour: number): string[] {
  return view.members.flatMap(member => member.runs.some(run => run.firstHour <= hour && hour <= run.lastHour) ? [tagAt(view, member, hour)] : []);
}

/** Who led at an hour; null when no leader was recorded then. */
export function leaderAt(leaders: readonly MembershipRun[], hour: number): string | null {
  return leaders.find(run => run.firstHour <= hour && hour <= run.lastHour)?.tag ?? null;
}

/** How many members at an hour; null when the roster is not known then. */
export function membersAt(view: FactionView, hour: number): number | null {
  if (view.unknown.some(span => span.firstHour <= hour && hour <= span.lastHour)) return null;
  return view.counts.find(step => step.firstHour <= hour && hour <= step.lastHour)?.value ?? 0;
}

/** A member's run cut where its leadership starts and ends, so a leader's stretch
 * is its own bar, marked. */
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

/** How the widget names things: countries and days, in the words it shows. */
export interface FactionText {
  country: (tag: string) => { name: string; flag?: string };
  day: (hour: number) => string;
}

export interface LaneOptions {
  order: FactionOrder;
  members: MemberOrder;
  /** Dissolved factions and members who left are shown. */
  ended: boolean;
  /** Only factions any of these countries ever belonged to; null for every faction. */
  countries: ReadonlySet<string> | null;
  /** Group members resolved at the hour their membership counts. */
  countryIds?: ReadonlySet<string> | null;
}

export const UNNAMED = UNNAMED_FACTION;

/** A faction's name as shown: the display name or the recorded one, else said to have none. */
export const factionLabel = (view: FactionView, hour?: number) => (hour == null ? view.name : view.nameAt(hour).name) ?? UNNAMED;

/** Words for a membership's state (`describeRun`). */
function memberWords(day: (hour: number) => string): RunWords {
  return {
    since: hour => `Joined ${day(hour)}`, before: hour => `Member before ${day(hour)}`,
    ended: hour => `Left ${day(hour)}`, endedBy: hour => `Left by ${day(hour)}`, lastSeen: hour => `Last recorded ${day(hour)}`,
  };
}

/** Words for a faction's existence. */
export function factionWords(day: (hour: number) => string): RunWords {
  return {
    since: hour => `Founded ${day(hour)}`, before: hour => `Founded before ${day(hour)}`,
    ended: hour => `Dissolved ${day(hour)}`, endedBy: hour => `Dissolved by ${day(hour)}`, lastSeen: hour => `Last recorded ${day(hour)}`,
  };
}

const firstHour = (runs: readonly HourRange[]) => runs[0]?.firstHour ?? Infinity;

/** What a lane is about: a faction, and for a member lane the member. */
export interface LaneInfo { faction: FactionView; member: MemberView | null }

/** A lane's id: structured, so no tag (a mod's tags may hold any character) can
 * blur two lanes, and never parsed back. A member's is its key (its country). */
export const laneId = (faction: string, member: string | null) => JSON.stringify(member == null ? ['faction', faction] : ['member', faction, member]);

/** The timeline's lanes at `hour` (the playhead, or the end with Live off): for each
 * faction founded by then, a heading lane with its member-count band, then a lane per
 * member who joined by then, and what each lane is about. Only what has started by
 * `hour` is listed, so lanes appear as playback reaches them. */
export function factionLanes(views: readonly FactionView[], hour: number, options: LaneOptions, text: FactionText,
  unobserved: readonly HourSpan[], branchFirstHour: number): { lanes: TimelineLane[]; info: Map<string, LaneInfo> } {
  const lanes: TimelineLane[] = [];
  const info = new Map<string, LaneInfo>();
  // Groups count tags at their membership hour; direct choices use the moment shown.
  const chosen = options.countryIds !== undefined ? options.countryIds : options.countries && views[0]
    ? new Set([...options.countries].map(tag => views[0].lineage.countryAt(tag, hour))) : null;
  const shown = views.filter(view => firstHour(view.existence) <= hour
    && (options.ended || runState(view.lifetime, hour, unobserved).kind !== 'ended')
    && (!chosen || view.members.some(member => chosen.has(member.key))));
  // Each faction's members at the hour, counted once rather than in every comparison.
  const counts = new Map(shown.map(view => [view, membersAt(view, hour) ?? -1]));
  const count = (view: FactionView) => counts.get(view)!;
  // Only name sorting needs a lookup map; other orders resolve in the heading.
  const names = options.order === 'name' ? new Map(shown.map(view => [view, factionLabel(view, hour)])) : null;
  shown.sort(options.order === 'name' ? (a, b) => names!.get(a)!.localeCompare(names!.get(b)!) || compareIds(a.id, b.id)
    : options.order === 'members' ? (a, b) => count(b) - count(a) || firstHour(a.existence) - firstHour(b.existence) || compareIds(a.id, b.id)
      : (a, b) => firstHour(a.existence) - firstHour(b.existence) || compareIds(a.id, b.id));
  for (const view of shown) {
    const name = names?.get(view) ?? factionLabel(view, hour);
    const exists = runState(view.lifetime, hour, unobserved).kind === 'held';
    const leader = leaderAt(view.leaders, hour);
    const recordedCount = count(view);
    const members = recordedCount < 0 ? null : recordedCount;
    const state = exists
      ? [leader ? `Led by ${text.country(leader).name}` : 'Leader not recorded', members == null ? 'members not recorded' : `${members} ${members === 1 ? 'member' : 'members'}`].join(' · ')
      : describeRun(view.lifetime, hour, factionWords(text.day), unobserved, branchFirstHour) ?? '';
    const headingId = laneId(view.id, null);
    info.set(headingId, { faction: view, member: null });
    lanes.push({
      id: headingId, heading: true, label: state, identity: { label: name }, icons: false, names: false,
      muted: !exists,
      spans: stretchesOf(view.counts).map(stretch => ({
        id: `${stretch.firstHour}`, firstHour: stretch.firstHour, lastHour: stretch.lastHour,
        label: name, color: view.color, band: stretch.steps,
      })),
    });
    const memberUnobserved = view.unknown;
    const shownMembers = view.members.filter(member => firstHour(member.runs) <= hour
      && (options.ended || runState(member.stretches, hour, memberUnobserved).kind !== 'ended'));
    // Each member's tag at the hour, found once.
    const tags = new Map(shownMembers.map(member => [member, tagAt(view, member, hour)]));
    const tagOf = (member: MemberView) => tags.get(member)!;
    const leading = (member: MemberView) => leader === tagOf(member) ? 0 : 1;
    const nameOf = (member: MemberView) => text.country(tagOf(member)).name;
    shownMembers.sort(options.members === 'name' ? (a, b) => nameOf(a).localeCompare(nameOf(b)) || compareTags(a.key, b.key)
      : (a, b) => firstHour(a.runs) - firstHour(b.runs) || leading(a) - leading(b) || nameOf(a).localeCompare(nameOf(b)) || compareTags(a.key, b.key));
    for (const member of shownMembers) {
      const tag = tagOf(member);
      const country = text.country(tag);
      const now = describeRun(member.stretches, hour, memberWords(text.day), memberUnobserved, branchFirstHour) ?? '';
      const ended = runState(member.stretches, hour, memberUnobserved).kind === 'ended';
      // Span ids need only be unique within their lane.
      const spans: TimelineSpan[] = member.runs.flatMap(run => leadershipParts(run, view.leaders).map(part => ({
        id: `${part.firstHour}`, firstHour: part.firstHour, lastHour: part.lastHour, label: country.name,
        color: view.color, ...(part.leader ? { glyph: LEADER_GLYPH } : {}),
      })));
      const id = laneId(view.id, member.key);
      info.set(id, { faction: view, member });
      lanes.push({
        id, depth: 1, identity: { label: country.name, icon: country.flag }, icons: false, names: false, muted: ended,
        label: leader === tag ? `${LEADER_GLYPH} Leader · ${now}` : now, spans,
      });
    }
  }
  return { lanes, info };
}

/** Recorded faction ids are integers: ordered numerically, so an order is stable. */
function compareIds(a: string, b: string): number {
  return a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);
}

/** The countries the given lanes name (a member, a faction's leader at `hour`), for
 * reading their names and flags. */
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

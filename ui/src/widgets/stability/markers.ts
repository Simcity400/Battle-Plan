import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { EventBadge } from '../../charts/badges';
import type { Artwork, CountryGroupReply, FactionGroup, FocusReply, HourSpan, IdeaHistoryReply, PartyHistoryReply } from '../../generated/dashboard';
import { factionNameAt, UNNAMED_FACTION } from '../../dashboard/factionNames.ts';
import type { FactionName } from '../../dashboard/factionNames';
import { provisionalGroupEvent } from '../../dashboard/countryCapitulations.ts';
import { preparedPending } from '../../dashboard/preparedPrefix.ts';
import { partyBadges } from '../ruling-party/partyData.ts';
import type { IdeaHistoryFirstView } from '../../dashboard/ideaHistoryReply.ts';

/** The event markers along the top of Stability & War support:
 * ⚔ wars and capitulations, ⚑ faction changes, ⚙
 * government changes (laws, advisors, ruling party) and ★ focuses completed.
 * Every event is dated as recorded; a start or end next to an unrecorded hour
 * is a gap, never an event. */

export type MarkerKind = 'wars' | 'factions' | 'government' | 'focus';

export const MARKER_GLYPHS: Record<MarkerKind, string> = { wars: '⚔', factions: '⚑', government: '⚙', focus: '★' };

export interface MarkerSources {
  tag: string;
  /** The hours shown: events outside them are left out. */
  extent: readonly [number, number];
  /** The branch's first and last hour: a span starting or ending there is not an event. */
  branch: readonly [number, number];
  /** Another country's name as the game showed it at an event's hour. */
  countryName: (tag: string, hour: number) => string;
  /** Whether the country is in the widget's group at an hour; its events outside it are left out. */
  member?: (hour: number) => boolean;
  groups?: CountryGroupReply;
  /** Campaign display names can replace the game's dated faction name. */
  factionName?: (faction: FactionGroup, hour: number) => FactionName;
  ideas?: IdeaHistoryFirstView;
  parties?: PartyHistoryReply;
  focus?: FocusReply;
  /** Scoped titles resolved at each completion's recorded observation hour. */
  focusName?: (token: string, hour: number) => string | null | undefined;
}

const inSpans = (spans: readonly HourSpan[], hour: number) => spans.some(span => span.firstHour <= hour && hour <= span.lastHour);

/** The markers of the kinds shown, in hour order. */
export function stabilityMarkers(sources: MarkerSources, kinds: ReadonlySet<MarkerKind>): EventBadge[] {
  const { tag, extent, branch, countryName } = sources;
  const within = (hour: number) => hour >= extent[0] && hour <= extent[1] && (sources.member?.(hour) ?? true);
  const badges: EventBadge[] = [];
  // A label is built only for an event shown, so only those ask for names.
  const push = (kind: MarkerKind, hour: number, label: string | (() => string), estimated = false) => {
    if (kinds.has(kind) && within(hour)) badges.push({ hour, glyph: MARKER_GLYPHS[kind], label: typeof label === 'string' ? label : label(),
      ...(estimated || kind === 'wars' && provisionalGroupEvent(sources.groups, hour) ? { approximate: true } : {}) });
  };
  const groups = sources.groups;
  if (groups && kinds.has('wars')) {
    // A war relation starts when a country declares or joins as an ally; the
    // recording does not say which, so the marker says only that it began.
    for (const war of groups.warStarts) {
      if (war.attacking === tag) push('wars', war.hour, () => `War with ${countryName(war.defending, war.hour)} began`);
      else if (war.defending === tag) push('wars', war.hour, () => `War with ${countryName(war.attacking, war.hour)} began`);
    }
    // This country's wars, by enemy, gathered once.
    const enemies = new Map<string, CountryGroupReply['wars']>();
    for (const war of groups.wars) {
      const enemy = war.first === tag ? war.second : war.second === tag ? war.first : null;
      if (enemy == null) continue;
      const list = enemies.get(enemy);
      if (list) list.push(war); else enemies.set(enemy, [war]);
    }
    for (const event of groups.capitulations) {
      if (event.tag === tag) { push('wars', event.hour, 'Capitulated'); continue; }
      // An enemy's capitulation: a country this one was at war with at that hour, or
      // the hour its war ended, when that hour's wars were recorded.
      // A war ending the hour before also counts under the tag the enemy had then,
      // when its tag changed that very hour (the war is recorded under the old tag).
      const formerTags = groups.identityChanges.filter(change => change.hour === event.hour && change.to === event.tag).map(change => change.from);
      const endsNow = (war: CountryGroupReply['wars'][number]) => event.hour === war.lastHour + 1 && !inSpans(groups.unobserved.wars, event.hour);
      const enemy = (enemies.get(event.tag) ?? []).some(war => war.firstHour <= event.hour && (event.hour <= war.lastHour || endsNow(war)))
        || formerTags.some(former => (enemies.get(former) ?? []).some(endsNow));
      if (enemy) push('wars', event.hour, () => `${countryName(event.tag, event.hour)} capitulated`);
    }
  }
  if (groups && kinds.has('factions')) {
    // Hours no faction was read, or still being prepared: beside them a run's edge is a gap.
    const gaps = [...groups.unobserved.factions, ...preparedPending(groups)];
    // A run by tag that starts or ends where the country's id changed tag, with
    // the other tag's run in the same faction meeting it there, is the same
    // country under another tag: not a join or a leave.
    for (const faction of groups.factions) {
      const nameAt = (hour: number) => {
        const named = sources.factionName?.(faction, hour) ?? factionNameAt(faction, hour);
        return named.name ?? UNNAMED_FACTION;
      };
      const endsAt = new Set(faction.members.map(member => `${member.lastHour + 1}#${member.tag}`));
      const startsAt = new Set(faction.members.map(member => `${member.firstHour}#${member.tag}`));
      const renamedTo = new Set(groups.identityChanges.filter(change => endsAt.has(`${change.hour}#${change.from}`)).map(change => `${change.hour}#${change.to}`));
      const renamedFrom = new Set(groups.identityChanges.filter(change => startsAt.has(`${change.hour}#${change.to}`)).map(change => `${change.hour}#${change.from}`));
      // Only a faction's sole first member founded it: the recording has no
      // founder, so several members seen at once all merely joined, and a first
      // roster that was not complete names no founder.
      const founders = faction.members.filter(member => member.firstHour === faction.firstHour);
      const unknown = [...gaps, ...faction.incomplete];
      const sole = founders.length === 1 && !inSpans(unknown, faction.firstHour);
      for (const run of faction.members.filter(member => member.tag === tag)) {
        const seenJoining = run.firstHour > branch[0] && !inSpans(unknown, run.firstHour - 1) && !renamedTo.has(`${run.firstHour}#${tag}`);
        const founded = run.firstHour === faction.firstHour && sole;
        if (seenJoining) push('factions', run.firstHour, () => {
          const name = nameAt(run.firstHour);
          return founded ? `Founded ${name}` : `Joined ${name}`;
        });
        const seenLeaving = run.lastHour < branch[1] && !inSpans(unknown, run.lastHour + 1) && !renamedFrom.has(`${run.lastHour + 1}#${tag}`);
        if (seenLeaving) push('factions', run.lastHour + 1, () => {
          // If it dissolved with the departure, its last membership read names what was left.
          const hour = run.lastHour + 1 > faction.lastHour ? run.lastHour : run.lastHour + 1;
          return `Left ${nameAt(hour)}`;
        });
      }
    }
  }
  const ideas = sources.ideas;
  if (ideas) {
    // A law is the idea itself, not its slot: a law slot may hold designers too.
    const laws = new Set(ideas.categories.flatMap(category => category.slots.flatMap(slot => slot.laws)));
    const seats = new Set(ideas.categories.flatMap(category => category.slots.filter(slot => slot.character).map(slot => slot.token)));
    const infos = new Map(ideas.ideas.map(info => [info.token, info]));
    const names = new Map<string, Artwork>(ideas.artwork.map(item => [item.token, item]));
    // A holder is its character as recorded at the time: script token and saved name.
    const holder = (token: string, character: number, script: string, name: string) => JSON.stringify([token, character, script, name]);
    const holders = new Map(ideas.holders.map(item => [holder(item.token, item.character, item.characterToken, item.name), item.artwork.name]));
    // What continues across hours: a character in its seat (whatever role idea it
    // holds there), otherwise the idea in its slot.
    const holding = (span: IdeaHistoryReply['spans'][number], hour: number) => span.character
      ? `character#${span.slot}#${span.character.id}#${hour}` : `idea#${span.token}#${span.slot}#${hour}`;
    const endings = new Set(ideas.spans.map(span => holding(span, span.lastHour + 1)));
    // Ideas held when the country's identity starts are its initial state, not changes.
    const starts = new Set(ideas.identityStarts);
    for (const span of ideas.spans) {
      // Taken on while recorded: not at the branch start, not after an unrecorded hour,
      // not as a new identity's initial state.
      if (span.firstHour <= branch[0] || inSpans(ideas.unobserved, span.firstHour - 1) || starts.has(span.firstHour)) continue;
      const slot = span.slot ?? infos.get(span.token)?.slot ?? null;
      // The same idea or character continuing right after (a change of country
      // identity, of a holder's details or of its role) is no change: a law kept,
      // the same character in the same seat.
      if (endings.has(holding(span, span.firstHour))) continue;
      // Beyond prepared coverage, lanes are sampled from keyframes, so the hour is only an estimate.
      const estimated = inSpans(ideas.approximate ?? [], span.firstHour);
      // Ideas are read at intervals, so the hour is the first read that saw the change: a bound.
      if (infos.get(span.token)?.law === true || laws.has(span.token)) push('government', span.firstHour, `Law: ${contentTitle(names.get(span.token)?.name, span.token, 'Idea name unavailable')} (taken on by then)`, estimated);
      else if (slot && seats.has(slot)) {
        // A holder the recording could not identify stays unidentified.
        const name = span.character ? holders.get(holder(span.token, span.character.id, span.character.token, span.character.name)) ?? 'holder not recorded'
          : contentTitle(names.get(span.token)?.name, span.token, 'Idea name unavailable');
        push('government', span.firstHour, `Advisor: ${name} (appointed by then)`, estimated);
      }
    }
  }
  if (sources.parties) {
    for (const badge of partyBadges(sources.parties, token => contentTitle(sources.parties!.ideologies.find(item => item.token === token)?.name, token, 'Ideology name unavailable'))) {
      if (badge.glyph !== '✓') push('government', badge.hour, badge.label);
    }
  }
  const focus = sources.focus;
  if (focus) {
    const names = new Map((focus.staticText ?? []).map(item => [item.token, item.name]));
    // Each completion once, by occurrence (a repeatable focus completed twice, even
    // seen at the same hour, is two events). The hour is the first read that saw it
    // completed, and reads are not hourly, so it is always a bound, never exact.
    const seen = new Set<string>();
    for (const segment of focus.history.segments) {
      for (const entry of segment.order.completed) {
        const occurrence = `${entry.token}#${entry.ordinal}#${entry.completedByHour}`;
        if (entry.completedByHour == null || seen.has(occurrence)) continue;
        seen.add(occurrence);
        const hour = entry.completedByHour;
        push('focus', hour, () => `Focus: ${contentTitle(sources.focusName?.(entry.token, hour) ?? names.get(entry.token), entry.token, 'Focus name unavailable')} (completed by then)`);
      }
    }
  }
  return badges.sort((a, b) => a.hour - b.hour || a.label.localeCompare(b.label));
}

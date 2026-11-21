import type { Failure } from '../../dashboard/queryError';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { ideaName } from '../../dashboard/ideaHistoryReply.ts';
import type { EventBadge } from '../../charts/badges';
import type { Artwork, CountryGroupReply, FactionGroup, FocusEntry, FocusReply, HourSpan, IdeaHistoryReply, PartyHistoryReply } from '../../generated/dashboard';
import { factionNameAt, UNNAMED_FACTION } from '../../dashboard/factionNames.ts';
import type { FactionName } from '../../dashboard/factionNames';
import { provisionalGroupEvent } from '../../dashboard/countryCapitulations.ts';
import { partyBadges } from '../ruling-party/partyData.ts';


export type MarkerKind = 'wars' | 'factions' | 'government' | 'focus';

export const MARKER_GLYPHS: Record<MarkerKind, string> = { wars: '⚔', factions: '⚑', government: '⚙', focus: '★' };

export interface MarkerSources {
  tag: string;
  extent: readonly [number, number];
  branch: readonly [number, number];
  countryName: (tag: string, hour: number) => string | Failure;
  member?: (hour: number) => boolean;
  groups?: CountryGroupReply;
  factionName?: (faction: FactionGroup, hour: number) => FactionName;
  ideas?: IdeaHistoryReply;
  parties?: PartyHistoryReply;
  focus?: FocusMarks;
  focusName?: (token: string, hour: number) => string | null | undefined | Failure;
}

export interface FocusMarks { completed: readonly FocusEntry[]; staticText?: readonly Artwork[] }

export function historyMarks(reply: FocusReply): FocusMarks {
  const seen = new Set<string>(), completed: FocusEntry[] = [];
  for (const segment of reply.history.segments) for (const entry of segment.order.completed) {
    const occurrence = `${entry.token}#${entry.ordinal}#${entry.completedByHour}`;
    if (!seen.has(occurrence)) { seen.add(occurrence); completed.push(entry); }
  }
  return { completed, staticText: reply.staticText };
}

const inSpans = (spans: readonly HourSpan[], hour: number) => spans.some(span => span.firstHour <= hour && hour <= span.lastHour);

export function stabilityMarkers(sources: MarkerSources, kinds: ReadonlySet<MarkerKind>): EventBadge[] {
  const { tag, extent, branch } = sources;
  let nameFailure: Failure | undefined;
  const countryName = (tag: string, hour: number) => {
    const name = sources.countryName(tag, hour);
    if (name instanceof Error) nameFailure = name;
    return contentTitle(name, tag, 'Country name unavailable', false, 'Couldn’t load country name.');
  };
  const within = (hour: number) => hour >= extent[0] && hour <= extent[1] && (sources.member?.(hour) ?? true);
  const badges: EventBadge[] = [];
  const push = (kind: MarkerKind, hour: number, label: string | (() => string | Pick<EventBadge, 'label' | 'failure'>), estimated = false) => {
    if (kinds.has(kind) && within(hour)) {
      nameFailure = undefined;
      const text = typeof label === 'string' ? label : label();
      badges.push({ hour, glyph: MARKER_GLYPHS[kind], ...(typeof text === 'string' ? { label: text } : text), ...(nameFailure ? { failure: nameFailure } : {}),
        ...(estimated || kind === 'wars' && provisionalGroupEvent(sources.groups, hour) ? { approximate: true } : {}) });
    }
  };
  const groups = sources.groups;
  if (groups && kinds.has('wars')) {
    for (const war of groups.warStarts) {
      if (war.attacking === tag) push('wars', war.hour, () => `War with ${countryName(war.defending, war.hour)} began`);
      else if (war.defending === tag) push('wars', war.hour, () => `War with ${countryName(war.attacking, war.hour)} began`);
    }
    const enemies = new Map<string, CountryGroupReply['wars']>();
    for (const war of groups.wars) {
      const enemy = war.first === tag ? war.second : war.second === tag ? war.first : null;
      if (enemy == null) continue;
      const list = enemies.get(enemy);
      if (list) list.push(war); else enemies.set(enemy, [war]);
    }
    for (const event of groups.capitulations) {
      if (event.tag === tag) { push('wars', event.hour, 'Capitulated'); continue; }
      const formerTags = groups.identityChanges.filter(change => change.hour === event.hour && change.to === event.tag).map(change => change.from);
      const endsNow = (war: CountryGroupReply['wars'][number]) => event.hour === war.lastHour + 1 && !inSpans(groups.unobserved.wars, event.hour);
      const enemy = (enemies.get(event.tag) ?? []).some(war => war.firstHour <= event.hour && (event.hour <= war.lastHour || endsNow(war)))
        || formerTags.some(former => (enemies.get(former) ?? []).some(endsNow));
      if (enemy) push('wars', event.hour, () => `${countryName(event.tag, event.hour)} capitulated`);
    }
  }
  if (groups && kinds.has('factions')) {
    const gaps = groups.unobserved.factions;
    for (const faction of groups.factions) {
      const nameAt = (hour: number) => {
        const named = sources.factionName?.(faction, hour) ?? factionNameAt(faction, hour);
        return named.name ?? UNNAMED_FACTION;
      };
      const endsAt = new Set(faction.members.map(member => `${member.lastHour + 1}#${member.tag}`));
      const startsAt = new Set(faction.members.map(member => `${member.firstHour}#${member.tag}`));
      const renamedTo = new Set(groups.identityChanges.filter(change => endsAt.has(`${change.hour}#${change.from}`)).map(change => `${change.hour}#${change.to}`));
      const renamedFrom = new Set(groups.identityChanges.filter(change => startsAt.has(`${change.hour}#${change.to}`)).map(change => `${change.hour}#${change.from}`));
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
          const hour = run.lastHour + 1 > faction.lastHour ? run.lastHour : run.lastHour + 1;
          return `Left ${nameAt(hour)}`;
        });
      }
    }
  }
  const ideas = sources.ideas;
  if (ideas) {
    const laws = new Set(ideas.categories.flatMap(category => category.slots.flatMap(slot => slot.laws)));
    const seats = new Set(ideas.categories.flatMap(category => category.slots.filter(slot => slot.character).map(slot => slot.token)));
    const infos = new Map(ideas.ideas.map(info => [info.token, info]));
    const names = new Map<string, Artwork>(ideas.artwork.map(item => [item.token, item]));
    const holder = (token: string, character: number, script: string, name: string) => JSON.stringify([token, character, script, name]);
    const holders = new Map(ideas.holders.map(item => [holder(item.token, item.character, item.characterToken, item.name), item.artwork.name]));
    const holding = (span: IdeaHistoryReply['spans'][number], hour: number) => span.character
      ? `character#${span.slot}#${span.character.id}#${hour}` : `idea#${span.token}#${span.slot}#${hour}`;
    const endings = new Set(ideas.spans.map(span => holding(span, span.lastHour + 1)));
    const starts = new Set(ideas.identityStarts);
    for (const span of ideas.spans) {
      if (span.firstHour <= branch[0] || inSpans(ideas.unobserved, span.firstHour - 1) || starts.has(span.firstHour)) continue;
      const slot = span.slot ?? infos.get(span.token)?.slot ?? null;
      if (endings.has(holding(span, span.firstHour))) continue;
      if (infos.get(span.token)?.law === true || laws.has(span.token)) push('government', span.firstHour, `Law: ${ideaName(names.get(span.token), infos.get(span.token), 'Idea name unavailable')} (taken on by then)`);
      else if (slot && seats.has(slot)) {
        const name = span.character ? holders.get(holder(span.token, span.character.id, span.character.token, span.character.name)) ?? 'holder not recorded'
          : ideaName(names.get(span.token), infos.get(span.token), 'Idea name unavailable');
        push('government', span.firstHour, `Advisor: ${name} (appointed by then)`);
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
    const seen = new Set<string>();
    for (const entry of focus.completed) {
      const occurrence = `${entry.token}#${entry.ordinal}#${entry.completedByHour}`;
      if (entry.completedByHour == null || seen.has(occurrence)) continue;
      seen.add(occurrence);
      const hour = entry.completedByHour;
      push('focus', hour, () => {
        const known = names.get(entry.token) ?? null;
        const read = known === null ? sources.focusName?.(entry.token, hour) : undefined;
        const name = known ?? read;
        return { label: `Focus: ${contentTitle(name, entry.token, 'Focus name unavailable', known === null && read === undefined && sources.focusName !== undefined, 'Couldn’t load focus name.')} (completed by then)`,
          ...(name instanceof Error ? { failure: name } : {}) };
      });
    }
  }
  return badges.sort((a, b) => a.hour - b.hour || a.label.localeCompare(b.label));
}

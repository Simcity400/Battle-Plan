import { inSpans, spanAt } from '../../charts/timeline/layout.ts';
import type { TimelineLane } from '../../charts/timeline/layout';
import type { Artwork, HourSpan, IdeaHistoryReply, IdeaSpan } from '../../generated/dashboard';

/** National spirits from the idea history: the ideas whose content category is a
 * national-spirit category (`type = national_spirit` in content, whatever a mod
 * names it). */

export type SpiritSort = 'gained' | 'ended' | 'name';

export interface Spirit {
  token: string;
  name: string;
  icon?: string;
  spans: IdeaSpan[];
}

export function spirits(reply: IdeaHistoryReply): Spirit[] {
  const spiritCategories = new Set(reply.categories.filter(category => category.kind === 'national_spirit').map(category => category.token));
  const tokens = new Set(reply.ideas.filter(info => info.category && spiritCategories.has(info.category)).map(info => info.token));
  const art = new Map<string, Artwork>(reply.artwork.map(item => [item.token, item]));
  const byToken = new Map<string, Spirit>();
  for (const span of reply.spans) {
    if (!tokens.has(span.token)) continue;
    let spirit = byToken.get(span.token);
    if (!spirit) {
      spirit = { token: span.token, name: art.get(span.token)?.name ?? span.token, icon: art.get(span.token)?.icon ?? undefined, spans: [] };
      byToken.set(span.token, spirit);
    }
    spirit.spans.push(span);
  }
  for (const spirit of byToken.values()) spirit.spans.sort((a, b) => a.firstHour - b.firstHour);
  return [...byToken.values()];
}

const asTimeline = (span: IdeaSpan) => ({ id: `${span.token}:${span.firstHour}`, firstHour: span.firstHour, lastHour: span.lastHour, label: span.token });

/** The span a spirit holds at an hour, if any. */
const heldAt = (spirit: Spirit, hour: number) => spanAt(spirit.spans.map(asTimeline), hour);

/** Spirits held at an hour, in the order they were last gained (the game's order). */
export function activeAt(list: readonly Spirit[], hour: number): Spirit[] {
  return list.filter(spirit => heldAt(spirit, hour) != null)
    .sort((a, b) => heldAt(a, hour)!.firstHour - heldAt(b, hour)!.firstHour || a.name.localeCompare(b.name));
}

const firstGained = (spirit: Spirit) => spirit.spans[0]?.firstHour ?? Infinity;

/** A spirit's state at an hour: held (since when), ended (when; with `lastSeen`
 * when the end fell in an unrecorded stretch, so only "by" that hour is known),
 * unknown (the hours after it was last seen are not recorded yet: last seen when),
 * or not yet gained. */
export type SpiritState = { kind: 'held'; since: number } | { kind: 'ended'; at: number; lastSeen?: number } | { kind: 'unknown'; lastSeen: number } | { kind: 'none' };

export function spiritState(spirit: Spirit, hour: number, unobserved: readonly HourSpan[] = []): SpiritState {
  const last = spirit.spans.filter(span => span.firstHour <= hour).at(-1);
  if (!last) return { kind: 'none' };
  if (hour < last.lastHour + 1) return { kind: 'held', since: last.firstHour };
  // A spirit whose next hours were not recorded may still be held: unknown, not an
  // end, until the first recorded hour after them shows it gone.
  const gap = unobserved.find(span => span.firstHour <= last.lastHour + 1 && last.lastHour + 1 <= span.lastHour);
  if (gap) return hour <= gap.lastHour ? { kind: 'unknown', lastSeen: last.lastHour } : { kind: 'ended', at: gap.lastHour + 1, lastSeen: last.lastHour };
  return { kind: 'ended', at: last.lastHour + 1 };
}

/** When a spirit ended as of `hour`, or null while held, unknown or not yet gained. */
export function endedBy(spirit: Spirit, hour: number, unobserved: readonly HourSpan[] = []): number | null {
  const state = spiritState(spirit, hour, unobserved);
  return state.kind === 'ended' ? state.at : null;
}

/** Whether nothing was recorded the hour before a span starts (the branch start
 * or an unrecorded stretch): the spirit was then gained at an unknown time up to
 * that hour, not at it. */
export function startUnseen(firstHour: number, branchFirstHour: number, unobserved: readonly HourSpan[] = []): boolean {
  return firstHour <= branchFirstHour || inSpans(unobserved, firstHour - 1);
}

/** One lane per spirit gained by `hour`: the spirit named in the left column, its
 * spans as bars (accented while still held), sorted as chosen. */
export function historyLanes(list: readonly Spirit[], hour: number, sort: SpiritSort, showEnded: boolean,
  describe: { since: (hour: number) => string; before: (hour: number) => string; ended: (hour: number) => string; endedBy: (hour: number) => string; lastSeen: (hour: number) => string },
  unobserved: readonly HourSpan[] = [], branchFirstHour = Number.NEGATIVE_INFINITY): TimelineLane[] {
  const seen = list.filter(spirit => firstGained(spirit) <= hour && (showEnded || endedBy(spirit, hour, unobserved) == null));
  const key = (spirit: Spirit) => endedBy(spirit, hour, unobserved) ?? Infinity;
  seen.sort(sort === 'name' ? (a, b) => a.name.localeCompare(b.name)
    : sort === 'ended' ? (a, b) => key(a) - key(b) || firstGained(a) - firstGained(b)
      : (a, b) => firstGained(a) - firstGained(b) || a.name.localeCompare(b.name));
  return seen.map(spirit => {
    const state = spiritState(spirit, hour, unobserved);
    const start = state.kind === 'held' ? state.since : firstGained(spirit);
    return {
      id: spirit.token,
      label: state.kind === 'ended' ? (state.lastSeen == null ? describe.ended(state.at) : describe.endedBy(state.at)) : state.kind === 'unknown' ? describe.lastSeen(state.lastSeen)
        : startUnseen(start, branchFirstHour, unobserved) ? describe.before(start) : describe.since(start),
      identity: { label: spirit.name, icon: spirit.icon },
      icons: false,
      names: false,
      spans: spirit.spans.map(span => ({ ...asTimeline(span), label: spirit.name, icon: spirit.icon, emphasis: span.firstHour <= hour && hour < span.lastHour + 1 })),
    };
  });
}

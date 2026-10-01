import { spanAt } from '../../charts/timeline/layout.ts';
import type { TimelineLane } from '../../charts/timeline/layout';
import { describeRun, runState, startUnseen } from '../../charts/timeline/runState.ts';
import type { RunState, RunWords } from '../../charts/timeline/runState';
import type { Artwork, HourSpan, IdeaHistoryReply, IdeaSpan } from '../../generated/dashboard';

/** National spirits from the idea history: the ideas whose content category is a
 * national-spirit category (`type = national_spirit` in content, whatever a mod
 * names it). `national_spirit` is the engine's category type keyword:
 * `CIdeaCategory::vf4@140FC0730` maps its token (0x3443) to type 1, beside
 * `army_spirit`, `navy_spirit` and `air_spirit`. */
export const NATIONAL_SPIRIT_TYPE = 'national_spirit';

export type SpiritSort = 'gained' | 'ended' | 'name';

export interface Spirit {
  token: string;
  name: string;
  icon?: string;
  spans: IdeaSpan[];
}

export function spirits(reply: IdeaHistoryReply): Spirit[] {
  const spiritCategories = new Set(reply.categories.filter(category => category.kind === NATIONAL_SPIRIT_TYPE).map(category => category.token));
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

/** A spirit's state at an hour, by the timeline kit's rule (`runState`): held,
 * ended (or ended by a day), unknown after its last recorded hour, or not yet gained. */
export type SpiritState = RunState;

export function spiritState(spirit: Spirit, hour: number, unobserved: readonly HourSpan[] = []): SpiritState {
  return runState(spirit.spans, hour, unobserved);
}

/** When a spirit ended as of `hour`, or null while held, unknown or not yet gained. */
export function endedBy(spirit: Spirit, hour: number, unobserved: readonly HourSpan[] = []): number | null {
  const state = spiritState(spirit, hour, unobserved);
  return state.kind === 'ended' ? state.at : null;
}

/** Whether a spirit was gained at an unknown time up to `firstHour` (the kit's `startUnseen`). */
export { startUnseen };

/** One lane per spirit gained by `hour`: the spirit named in the left column, its
 * spans as bars (accented while still held), sorted as chosen. */
export function historyLanes(list: readonly Spirit[], hour: number, sort: SpiritSort, showEnded: boolean,
  describe: RunWords, unobserved: readonly HourSpan[] = [], branchFirstHour = Number.NEGATIVE_INFINITY): TimelineLane[] {
  const seen = list.filter(spirit => firstGained(spirit) <= hour && (showEnded || endedBy(spirit, hour, unobserved) == null));
  const key = (spirit: Spirit) => endedBy(spirit, hour, unobserved) ?? Infinity;
  seen.sort(sort === 'name' ? (a, b) => a.name.localeCompare(b.name)
    : sort === 'ended' ? (a, b) => key(a) - key(b) || firstGained(a) - firstGained(b)
      : (a, b) => firstGained(a) - firstGained(b) || a.name.localeCompare(b.name));
  return seen.map(spirit => {
    return {
      id: spirit.token,
      label: describeRun(spirit.spans, hour, describe, unobserved, branchFirstHour) ?? '',
      identity: { label: spirit.name, icon: spirit.icon },
      icons: false,
      names: false,
      spans: spirit.spans.map(span => ({ ...asTimeline(span), label: spirit.name, icon: spirit.icon, emphasis: span.firstHour <= hour && hour < span.lastHour + 1 })),
    };
  });
}

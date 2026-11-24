import { ideaName, ideaNameMissing } from '../../dashboard/ideaHistoryReply.ts';
import { spanAt } from '../../charts/timeline/layout.ts';
import { runState, startUnseen } from '../../charts/timeline/runState.ts';
import type { RunState } from '../../charts/timeline/runState';
import type { Artwork, HourSpan, IdeaHistoryReply, IdeaInfo, IdeaSpan } from '../../generated/dashboard';

export const NATIONAL_SPIRIT_TYPE = 'national_spirit';

export interface Spirit {
  token: string;
  name: string;
  nameMissing: string | null;
  icon?: string;
  spans: IdeaSpan[];
}

export function spirits(reply: IdeaHistoryReply): Spirit[] {
  const spiritCategories = new Set(reply.categories.filter(category => category.kind === NATIONAL_SPIRIT_TYPE).map(category => category.token));
  const infos = new Map<string, IdeaInfo>(reply.ideas.filter(info => info.category && spiritCategories.has(info.category)).map(info => [info.token, info]));
  const art = new Map<string, Artwork>(reply.artwork.map(item => [item.token, item]));
  const byToken = new Map<string, Spirit>();
  for (const span of reply.spans) {
    const info = infos.get(span.token);
    if (!info) continue;
    let spirit = byToken.get(span.token);
    if (!spirit) {
      spirit = { token: span.token, name: ideaName(art.get(span.token), info, 'National spirit name unavailable'),
        nameMissing: ideaNameMissing(art.get(span.token), info), icon: art.get(span.token)?.icon ?? undefined, spans: [] };
      byToken.set(span.token, spirit);
    }
    spirit.spans.push(span);
  }
  for (const spirit of byToken.values()) spirit.spans.sort((a, b) => a.firstHour - b.firstHour);
  return [...byToken.values()];
}

const asTimeline = (span: IdeaSpan) => ({ id: `${span.token}:${span.firstHour}`, firstHour: span.firstHour, lastHour: span.lastHour, label: span.token });

const heldAt = (spirit: Spirit, hour: number) => spanAt(spirit.spans.map(asTimeline), hour);

export function activeAt(list: readonly Spirit[], hour: number): Spirit[] {
  return list.filter(spirit => heldAt(spirit, hour) != null)
    .sort((a, b) => heldAt(a, hour)!.firstHour - heldAt(b, hour)!.firstHour || a.name.localeCompare(b.name));
}

const firstGained = (spirit: Spirit) => spirit.spans[0]?.firstHour ?? Infinity;

export type SpiritState = RunState;

export function spiritState(spirit: Spirit, hour: number, unobserved: readonly HourSpan[] = []): SpiritState {
  return runState(spirit.spans, hour, unobserved);
}

export function endedBy(spirit: Spirit, hour: number, unobserved: readonly HourSpan[] = []): number | null {
  const state = spiritState(spirit, hour, unobserved);
  return state.kind === 'ended' ? state.at : null;
}

export { startUnseen };

export function spiritsShown(list: readonly Spirit[], hour: number, showEnded: boolean, unobserved: readonly HourSpan[] = []): Spirit[] {
  return list.filter(spirit => firstGained(spirit) <= hour && (showEnded || endedBy(spirit, hour, unobserved) == null));
}

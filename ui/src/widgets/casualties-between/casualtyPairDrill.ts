import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import type { CompactStackModel, StackRun } from '../../charts/stack';
import { runAt } from '../../charts/series.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { CasualtyPairsReply } from '../../generated/dashboard';
import { DETAIL } from './definition.ts';
import { pairFlowId } from './casualtyPairsData.ts';

export const casualtyPairIds = (replies: readonly CasualtyPairsReply[]) => [...new Set(replies.flatMap(reply => reply.pairs.flatMap(pair => [
  pairFlowId(pair.identity, 'first'), pairFlowId(pair.identity, 'second'),
])))];

/** A saved selection belongs to one campaign and branch, and keeps native holders,
 * not merely their reusable country tags. */
export function readPairSelection(value: unknown, track: Pick<BoundTrack, 'source' | 'branch'>): string[] | null {
  if (typeof value !== 'string' || !value) return null;
  try {
    const item = JSON.parse(contentKey(value));
    if (item.campaign !== track.source.metadata.campaignKey || item.branch !== track.branch.id || !Array.isArray(item.ids)
      || !item.ids.length || new Set(item.ids).size !== item.ids.length) return null;
    for (const id of item.ids) {
      if (typeof id !== 'string') return null;
      const parts = JSON.parse(id);
      if (!Array.isArray(parts) || parts.length !== 2 || typeof parts[0] !== 'string' || !parts[0]
        || parts[1] !== 'first' && parts[1] !== 'second' || JSON.stringify(parts) !== id) return null;
    }
    return item.ids;
  } catch { return null; }
}

export function pinPairs(widget: WidgetSpec, track: BoundTrack, ids: readonly string[], hour: number, following: boolean): WidgetSpec {
  const pinned = pinnedBreakdown(widget, DETAIL, track.id);
  const offset = Math.floor(hour) - track.branch.firstHour;
  return { ...pinned, time: following ? { mode: 'follow' } : { mode: 'pin', windows: {
    [track.id]: { firstOffset: offset, lastOffset: offset },
  } }, options: { ...pinned.options, pairSelection: setToken(JSON.stringify({ campaign: track.source.metadata.campaignKey,
    branch: track.branch.id, ids: [...new Set(ids)] })) } };
}

/** Positive geometry for the same directed native lifetimes. Outside an observed
 * block the stack breaks. An absent relation contributes no geometry; exact rows
 * still distinguish its absence from a recorded zero. */
export function pairHistory(replies: readonly CasualtyPairsReply[], ids: readonly string[]) {
  const source = new Map(ids.map(id => [id, [] as StackRun[]]));
  const observed = replies.flatMap(reply => reply.observed).sort((a, b) => a.firstHour - b.firstHour);
  const events = new Map<number, bigint>();
  const add = (hour: number, amount: bigint) => events.set(hour, (events.get(hour) ?? 0n) + amount);
  for (const span of observed) { add(span.firstHour, 0n); add(span.lastHour, 0n); add(span.lastHour + 1, 0n); }
  for (const reply of replies) for (const pair of reply.pairs) {
    for (const [direction, field] of [['first', 'secondTaken'], ['second', 'firstTaken']] as const) {
      const runs = source.get(pairFlowId(pair.identity, direction));
      if (!runs) continue;
      for (const run of pair.runs) {
        const value = BigInt(run[field]);
        runs.push({ firstHour: run.firstHour, lastHour: run.lastHour, value: value > 0n ? Number(value) : 0 });
        add(run.firstHour, value > 0n ? value : 0n);
        add(run.lastHour, 0n);
        add(run.lastHour + 1, value > 0n ? -value : 0n);
      }
    }
  }
  const model: CompactStackModel = { bands: ids.map(id => source.get(id)!.sort((a, b) => a.firstHour - b.firstHour)),
    hours: [...events.keys()].sort((a, b) => a - b), bounds: [], breakBefore: [] };
  let positive = 0n;
  for (const hour of model.hours) {
    positive += events.get(hour)!;
    model.bounds.push(ids.length && runAt(observed, hour) ? [0, Number(positive)] : null);
    model.breakBefore.push(false);
  }
  return model;
}

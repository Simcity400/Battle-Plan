import type { GatedTime } from '../../dashboard/regionTime';
import type { PinnedSpec } from '../../charts/drill/drillState';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import type { CompactStackModel, StackRun } from '../../charts/stack';
import { runAt } from '../../charts/series.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { CasualtyPairsReply } from './casualtyPairsData';
import { DETAIL } from './definition.ts';
import { casualtyPairsAt, exactCount, pairFlowId } from './casualtyPairsData.ts';
import { liveKey } from '../../charts/live.ts';

export const casualtyPairIds = (replies: readonly CasualtyPairsReply[]) => [...new Set(replies.flatMap(reply => reply.pairs.flatMap(pair => [
  pairFlowId(pair.identity, 'first'), pairFlowId(pair.identity, 'second'),
])))];

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

export function pinPairs(widget: WidgetSpec, track: BoundTrack, ids: readonly string[], time: GatedTime): PinnedSpec {
  const pinned = pinnedBreakdown(widget, DETAIL, track.id, time);
  return { ...pinned, options: { ...pinned.options, pairSelection: setToken(JSON.stringify({ campaign: track.source.metadata.campaignKey,
    branch: track.branch.id, ids: [...new Set(ids)] })) } };
}

const decoded = new WeakMap<CasualtyPairsReply, { id: string; run: StackRun; positive: bigint }[]>();
function directedRuns(reply: CasualtyPairsReply) {
  let runs = decoded.get(reply);
  if (!runs) {
    runs = reply.pairs.flatMap(pair => (['first', 'second'] as const).flatMap(direction => {
      const id = pairFlowId(pair.identity, direction);
      return pair.runs.map(run => {
        const value = BigInt(direction === 'first' ? run.secondTaken : run.firstTaken), positive = value > 0n ? value : 0n;
        return { id, positive, run: { firstHour: run.firstHour, lastHour: run.lastHour, value: Number(positive) } };
      });
    }));
    decoded.set(reply, runs);
  }
  return runs;
}

export function pairHistory(replies: readonly CasualtyPairsReply[], ids: readonly string[]) {
  const source = new Map(ids.map(id => [id, [] as StackRun[]]));
  const observed = replies.flatMap(reply => reply.observed).sort((a, b) => a.firstHour - b.firstHour);
  const events = new Map<number, bigint>();
  const add = (hour: number, amount: bigint) => events.set(hour, (events.get(hour) ?? 0n) + amount);
  for (const span of observed) { add(span.firstHour, 0n); add(span.lastHour, 0n); add(span.lastHour + 1, 0n); }
  for (const reply of replies) for (const directed of directedRuns(reply)) {
    const runs = source.get(directed.id);
    if (!runs) continue;
    const { run } = directed;
    runs.push(run);
    add(run.firstHour, directed.positive);
    add(run.lastHour, 0n);
    add(run.lastHour + 1, -directed.positive);
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

export function pairDrillAt(reply: CasualtyPairsReply | undefined, hour: number, selected: ReadonlySet<string>): {
  rows: string; values: Map<string, bigint | number>; total: string;
} {
  const flows = reply ? casualtyPairsAt(reply, hour).flows.filter(flow => selected.has(flow.id)) : [];
  const subtotal = flows.some(flow => flow.incomplete);
  const positive = flows.reduce((sum, flow) => sum + (flow.value > 0n ? flow.value : 0n), 0n);
  const values = new Map<string, bigint | number>();
  for (const flow of flows) {
    values.set(liveKey(flow.id, 'value'), flow.value);
    if (!subtotal && positive > 0n && flow.value > 0n) values.set(liveKey(flow.id, 'share'), Number(flow.value * 1000n / positive) / 10);
  }
  const total = `${subtotal ? 'Recorded subtotal' : 'Signed total'}: ${exactCount(flows.reduce((sum, flow) => sum + flow.value, 0n))}`;
  return { rows: flows.map(flow => `${flow.id}${flow.incomplete ? '!' : ''}`).sort().join(','), values, total };
}

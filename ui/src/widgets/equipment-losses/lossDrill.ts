import type { GatedTime } from '../../dashboard/regionTime';
import type { HeldCountry } from '../../dashboard/regionGate';
import type { PinnedSpec } from '../../charts/drill/drillState';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import { runAt } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { stackEdges } from '../../charts/stack.ts';
import { shareSegments } from '../../charts/share.ts';
import type { StackModel } from '../../charts/stack';
import type { DrillPart } from '../../charts/drill/parts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { entryAt } from '../../dashboard/reads.ts';
import type { LossesReply } from '../../generated/dashboard';
import { causeName, exactPointAt, lossDesignName, lossItems, lossKey, pointAt, pointIn, sourceKey, typeName } from './lossesData.ts';
import type { LossWindow } from './lossesData';
import { heldDrillCountry } from '../country-measure/drillCountry.ts';
import { DETAIL } from './definition.ts';

export function lossNumbers(reply: LossesReply): LossesReply {
  return { ...reply, warnings: [], causes: reply.causes.map(cause => ({ ...cause,
    name: { key: null, template: null, text: null }, group: null })),
    designs: reply.designs.map(design => ({ ...design, equipment: null, archetype: null, archetypeName: null,
      names: design.names.map(read => ({ ...read, name: null })) })) };
}

export interface LossRow {
  id: string; causeKey: string; cause: string; type: string; design: string; exact: bigint | null;
  readHour: number | null; incomplete: boolean; note?: string;
}

export function lossRows(reply: LossesReply, window: LossWindow, selected: ReadonlySet<string> | null = null): LossRow[] {
  const items = lossItems(reply, window);
  const byKey = new Map(items.flatMap(item => item.counted?.map(id => [id, item] as const) ?? []));
  const byCause = new Map(items.filter(item => item.path.length === 1).map(item => [item.path[0].id, item]));
  const rows: LossRow[] = [];
  for (const cause of reply.causes) {
    if (!cause.loss) continue;
    const causeItem = byCause.get(`cause:${sourceKey(cause.source)}`);
    if (cause.unknown !== null || causeItem?.unknownValue || causeItem?.incomplete && !cause.leaves.length) {
      const id = JSON.stringify([sourceKey(cause.source), 'unknown']);
      if (!selected || selected.has(id)) rows.push({ id, causeKey: sourceKey(cause.source), cause: causeName(cause), type: 'Not known', design: 'Not known',
        exact: null, readHour: null, incomplete: true, note: 'Unavailable' });
    }
    for (const leaf of cause.leaves) {
      const design = leaf.design === null ? undefined : reply.designs[leaf.design];
      const id = lossKey(cause, design);
      if (selected && !selected.has(id)) continue;
      const point = pointAt(leaf, window.hour), item = byKey.get(id);
      if (!point || !item || item.present === false) continue;
      rows.push({ id, causeKey: sourceKey(cause.source), cause: causeName(cause), type: typeName(design), design: lossDesignName(design, window.hour),
        exact: item?.exact ?? null, readHour: point.hour, incomplete: item?.incomplete === true,
        note: item?.note });
    }
  }
  return rows;
}

export function lossParts(rows: readonly LossRow[], scale: number): (DrillPart & { keys: string[]; incomplete: boolean })[] {
  const causes = new Map<string, { label: string; raw: bigint; measured: boolean; incomplete: boolean; keys: string[] }>();
  for (const row of rows) {
    let cause = causes.get(row.causeKey);
    if (!cause) { cause = { label: row.cause, raw: 0n, measured: false, incomplete: false, keys: [] }; causes.set(row.causeKey, cause); }
    cause.raw += row.exact ?? 0n;
    cause.measured ||= row.exact !== null;
    cause.incomplete ||= row.incomplete || row.exact === null;
    cause.keys.push(row.id);
  }
  return [...causes].map(([id, cause]) => ({ id, label: cause.label, raw: cause.measured ? cause.raw : null,
    value: cause.measured ? Number(cause.raw) / scale : null, keys: cause.keys, incomplete: cause.incomplete }));
}

export interface LossSelection { campaign: string; branch: string; tag: string; keys: string[] }
export function readLossSelection(value: unknown, campaign: string, branch: string, tag: string): LossSelection | null | undefined {
  if (value == null) return null;
  if (typeof value !== 'string') return undefined;
  try {
    const pick = JSON.parse(value) as LossSelection;
    return pick && pick.campaign === campaign && pick.branch === branch && pick.tag === tag
      && Array.isArray(pick.keys) && pick.keys.every(key => typeof key === 'string') ? pick : undefined;
  } catch { return undefined; }
}

export function pinLosses(widget: WidgetSpec, track: BoundTrack, held: HeldCountry, time: GatedTime, keys: readonly string[] | null): PinnedSpec {
  const country = heldDrillCountry(held, track);
  const pinned = pinnedBreakdown(widget, DETAIL, track.id, time);
  const options = { ...pinned.options };
  delete options.lossSelection;
  delete options.lossCountry;
  options.lossCountry = JSON.stringify(country);
  const pinnedTag = country.tag;
  if (keys) options.lossSelection = JSON.stringify({ campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag: pinnedTag, keys });
  return { ...pinned, countries: { kind: 'tags', tags: [pinnedTag] }, options };
}

export function lossSegments(model: StackModel, first: number, last: number, hoursPerPixel: number) {
  return shareSegments(model, first, last, hoursPerPixel);
}

function addLeafSegment(
  at: (hour: number) => { deltas: Map<string, bigint>; unread: number; runBreak: boolean; measured: number; absent: number },
  key: string,
  leaf: LossesReply['causes'][number]['leaves'][number],
  segFirst: number,
  segLast: number,
  minHour: number | null,
  baseline: bigint,
  resetAtEnd: boolean,
) {
  const hasExact = leaf.exact.some(point => point.hour >= segFirst && point.hour <= segLast);
  let previous = 0n;
  let wasMeasured = false;
  let wasAbsent = false;
  const setState = (hour: number, next: bigint, isMeasured: boolean, isAbsent: boolean) => {
    if (next !== previous || isMeasured !== wasMeasured || isAbsent !== wasAbsent) {
      const event = at(hour);
      if (next !== previous) {
        event.deltas.set(key, (event.deltas.get(key) ?? 0n) + next - previous);
        previous = next;
      }
      if (isMeasured !== wasMeasured) {
        event.measured += isMeasured ? 1 : -1;
        wasMeasured = isMeasured;
      }
      if (isAbsent !== wasAbsent) {
        event.absent += isAbsent ? 1 : -1;
        wasAbsent = isAbsent;
      }
    }
  };
  if (!hasExact) {
    const initial = pointIn(leaf, segFirst, minHour);
    if (initial) setState(segFirst, BigInt(initial.totalRaw) - baseline, true, false);
    for (const point of leaf.points) {
      if (point.hour <= segFirst || point.hour > segLast) continue;
      setState(point.hour, BigInt(point.totalRaw) - baseline, true, false);
    }
  } else {
    const hours = new Set<number>([segFirst]);
    for (const point of leaf.points) {
      if (point.hour > segFirst && point.hour <= segLast) hours.add(point.hour);
    }
    for (const point of leaf.exact) {
      if (point.hour < segFirst || point.hour > segLast) continue;
      hours.add(point.hour);
      if (point.totalRaw === null && point.hour < segLast) hours.add(point.hour + 1);
    }
    for (const hour of [...hours].sort((a, b) => a - b)) {
      const exact = exactPointAt(leaf, hour);
      if (exact && exact.totalRaw === null) {
        setState(hour, 0n, false, true);
      } else {
        const point = pointIn(leaf, hour, minHour);
        if (point) setState(hour, BigInt(point.totalRaw) - baseline, true, false);
        else setState(hour, 0n, false, false);
      }
    }
  }
  if (resetAtEnd || wasAbsent) {
    const endEvent = at(segLast + 1);
    if (resetAtEnd && previous !== 0n) endEvent.deltas.set(key, (endEvent.deltas.get(key) ?? 0n) - previous);
    if (resetAtEnd && wasMeasured) endEvent.measured -= 1;
    if (wasAbsent) endEvent.absent -= 1;
  }
}

export function lossHistory(reply: LossesReply, first: number, last: number, from?: number | null,
  selected: ReadonlySet<string> | null = null, countries?: readonly Span[]): { model: StackModel; keys: string[]; labels: string[] } {
  const events = new Map<number, { deltas: Map<string, bigint>; unread: number; runBreak: boolean; measured: number; absent: number }>();
  const at = (hour: number) => {
    let event = events.get(hour);
    if (!event) { event = { deltas: new Map(), unread: 0, runBreak: false, measured: 0, absent: 0 }; events.set(hour, event); }
    return event;
  };
  const keys: string[] = [], labels: string[] = [];
  let destruction = false;
  at(first); at(last + 1);
  for (const cause of reply.causes) {
    if (!cause.loss || cause.unknown !== null) continue;
    const key = sourceKey(cause.source);
    let included = false;
    const isDestruction = cause.source.kind === 'destruction';
    for (const leaf of cause.leaves) {
      const design = leaf.design === null ? undefined : reply.designs[leaf.design];
      if (selected && !selected.has(lossKey(cause, design))) continue;
      included = true;
      if (isDestruction) {
        for (const [index, run] of reply.destructionRuns.entries()) {
          if (run.countedFrom === null) continue;
          const runStart = Math.max(run.firstHour, run.countedFrom);
          const segFirst = Math.max(first, runStart);
          const segLast = Math.min(last, run.lastHour);
          if (segFirst > segLast) continue;
          if (index > 0 && segFirst === run.firstHour) at(segFirst).runBreak = true;
          const baseline = from != null && from > runStart ? BigInt(pointIn(leaf, from - 1, runStart)?.totalRaw ?? '0') : 0n;
          addLeafSegment(at, key, leaf, segFirst, segLast, runStart, baseline, true);
        }
        continue;
      }
      const baseline = from == null ? 0n : BigInt(pointAt(leaf, from - 1)?.totalRaw ?? '0');
      addLeafSegment(at, key, leaf, first, last, null, baseline, false);
    }
    if (!included) continue;
    keys.push(key); labels.push(causeName(cause));
    destruction ||= isDestruction;
    for (const span of cause.unknownSpans) if (span.reason === 'unread' && span.firstHour <= last && span.lastHour >= first) {
      at(Math.max(first, span.firstHour)).unread++;
      at(Math.min(last + 1, span.lastHour + 1)).unread--;
    }
  }
  if (destruction) for (const span of reply.destructionCoverage) {
    if (span.firstHour <= last && span.lastHour >= first) { at(Math.max(first, span.firstHour)); at(Math.min(last + 1, span.lastHour + 1)); }
  }
  for (const span of countries ?? []) if (span.firstHour <= last && span.lastHour >= first) {
    at(Math.max(first, span.firstHour)); at(Math.min(last + 1, span.lastHour + 1));
  }
  const model: StackModel = { hours: [...events.keys()].sort((a, b) => a - b), edges: [], breakBefore: [] };
  const amounts = new Map<string, bigint>();
  let unread = 0, measured = 0, absent = 0, broken = true;
  for (const hour of model.hours) {
    const event = events.get(hour)!;
    unread += event.unread;
    measured += event.measured;
    absent += event.absent;
    for (const [key, delta] of event.deltas) amounts.set(key, (amounts.get(key) ?? 0n) + delta);
    const drain = entryAt(reply.destructionCoverage, hour);
    const known = hour <= last && unread === 0 && !(absent > 0 && measured === 0) && (!countries || runAt(countries, hour) !== null) && (!destruction || drain !== null && drain.lastHour >= hour && drain.reason === null);
    model.edges.push(known ? stackEdges(keys.map(key => Math.max(0, Number(amounts.get(key) ?? 0n) / reply.amountScale))) : null);
    model.breakBefore.push(known && (broken || event.runBreak)); broken = !known;
  }
  return { model, keys, labels };
}

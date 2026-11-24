import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { Coverage, DestructionRun, ExactLossPoint, LossCause, LossDesign, LossLeaf, LossName, LossPoint, LossSource, LossesReply, NavigationKey, ShipListing, UnknownSpan } from '../../generated/dashboard';
import type { BreakdownLeafItem as SunburstItem, BreakdownLeafTotal as SunburstTotal } from '../../families/breakdown/tree';
import type { LegendItem } from '../../charts/Legend';
import type { ChartTokens } from '../../charts/tokens';
import { mix } from '../../shell/appearance/color.ts';
import { formatRate } from '../../charts/valueAxis.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { entryAt } from '../../dashboard/reads.ts';
import { int, object } from '../../dashboard/validation.ts';

export const LEVELS = ['causeLevel', 'typeLevel', 'designLevel'] as const;
export const UNKNOWN_LABELS = {
  unread: 'Cause not read at the moment shown',
  endedAfterLastRead: 'Exchange ended after its last read',
  destructionWithheld: 'Destruction table withheld',
  destructionGap: 'Destruction drain not recorded',
  destructionUnresolved: 'Destruction owner not resolved',
  destructionDomain: 'Destruction equipment arm not resolved',
};

const text = (value: unknown) => value === null || typeof value === 'string';
const RAW = /^-?\d{1,39}$/;
export const sourceKey = (source: LossSource) => source.kind === 'combatLog' || source.kind === 'destruction' ? `${source.kind}:${source.reason}` : source.kind;
const isSource = (source: LossSource) => object(source) && (source.kind === 'combatLog'
  ? int(source.reason) && source.reason >= 0 && source.reason <= 255
  : source.kind === 'destruction' ? typeof source.reason === 'string' && source.reason.length > 0
  : ['recovered', 'sunkShips', 'sunkConvoys', 'lendLeaseSunk', 'lendLeaseSent'].includes(source.kind));
const isName = (name: LossName) => object(name) && text(name.key) && text(name.template) && text(name.text);
const isCoverage = (coverage: Coverage) => ['logged', 'notLogged', 'unknown', 'pruneUnobserved'].includes(coverage as string)
  || (typeof coverage === 'object' && coverage !== null && 'partial' in coverage && object(coverage.partial) && int(coverage.partial.ungrouped) && coverage.partial.ungrouped > 0);
const isSpan = (span: UnknownSpan) => object(span) && int(span.firstHour) && int(span.lastHour) && span.firstHour <= span.lastHour
  && Object.hasOwn(UNKNOWN_LABELS, span.reason) && text(span.detail);
const isHours = (span: { firstHour: number; lastHour: number }) => object(span) && int(span.firstHour) && int(span.lastHour) && span.firstHour <= span.lastHour;
const isDestructionRun = (run: DestructionRun) => isHours(run)
  && (run.countedFrom === null || int(run.countedFrom)) && (run.withheldFrom === null || int(run.withheldFrom));
const isDesign = (design: LossDesign) => object(design) && int(design.variant) && design.variant >= 0
  && [design.equipment, design.archetype, design.archetypeName].every(text)
  && Array.isArray(design.names) && design.names.every((read, index) => object(read) && int(read.firstHour) && text(read.name)
    && typeof read.known === 'boolean'
    && (index === 0 || design.names[index - 1].firstHour < read.firstHour));
const isExactPoint = (point: ExactLossPoint) => object(point) && int(point.hour)
  && (point.totalRaw === null || typeof point.totalRaw === 'string' && RAW.test(point.totalRaw));
const isLeaf = (leaf: LossLeaf, designs: number) => object(leaf)
  && (leaf.design === null || (int(leaf.design) && leaf.design >= 0 && leaf.design < designs))
  && typeof leaf.key === 'string' && (leaf.lowerBoundFrom === null || int(leaf.lowerBoundFrom)) && Array.isArray(leaf.points)
  && leaf.points.every((point, index) => object(point) && int(point.hour) && typeof point.totalRaw === 'string' && RAW.test(point.totalRaw)
    && (index === 0 || leaf.points[index - 1].hour < point.hour))
  && Array.isArray(leaf.exact) && leaf.exact.every((point, index) => isExactPoint(point)
    && (index === 0 || leaf.exact[index - 1].hour < point.hour));

export function validateLossesReply(data: unknown): LossesReply {
  const reply = data as LossesReply;
  const ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && ['destruction', 'combatLog'].includes(reply.landSource) && text(reply.landLowerBound)
    && Array.isArray(reply.destructionRuns) && reply.destructionRuns.every((run, index) => isDestructionRun(run)
      && (index === 0 || reply.destructionRuns[index - 1].lastHour < run.firstHour))
    && int(reply.amountScale) && reply.amountScale > 0 && Number.isInteger(Math.log10(reply.amountScale))
    && (reply.retainedMonths === null || int(reply.retainedMonths))
    && Array.isArray(reply.designs) && reply.designs.every(isDesign)
    && new Set(reply.designs.map(design => design.variant)).size === reply.designs.length
    && Array.isArray(reply.coverage) && reply.coverage.every((span, index) => object(span) && int(span.firstHour) && int(span.lastHour)
      && span.firstHour <= span.lastHour && isCoverage(span.coverage) && (index === 0 || reply.coverage[index - 1].lastHour < span.firstHour))
    && Array.isArray(reply.destructionCoverage) && reply.destructionCoverage.every((span, index) => isHours(span) && text(span.reason)
      && (index === 0 || reply.destructionCoverage[index - 1].lastHour < span.firstHour))
    && Array.isArray(reply.causes) && reply.causes.every(cause => object(cause) && isSource(cause.source) && isName(cause.name)
      && ['land', 'air', 'naval', 'other'].includes(cause.domain)
      && (cause.group === null || isName(cause.group)) && typeof cause.loss === 'boolean' && text(cause.unknown)
      && Array.isArray(cause.unknownSpans) && cause.unknownSpans.every(isSpan)
      && Array.isArray(cause.leaves) && cause.leaves.every(leaf => isLeaf(leaf, reply.designs.length))
      && new Set(cause.leaves.map(leaf => leaf.design)).size === cause.leaves.length
      && (cause.unknown === null || cause.leaves.length === 0))
    && new Set(reply.causes.map(cause => sourceKey(cause.source))).size === reply.causes.length
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.ships) && reply.ships.every(ship => object(ship) && typeof ship.signature === 'string' && int(ship.listedHour) && int(ship.sinkHour)
      && int(ship.count) && ship.count > 0 && (ship.design === null || int(ship.design) && ship.design >= 0 && ship.design < reply.designs.length))
    && int(reply.from) && int(reply.to) && reply.from <= reply.to && int(reply.width) && reply.width > 0
    && Array.isArray(reply.exactHours) && reply.exactHours.every(int) && Array.isArray(reply.restarts) && reply.restarts.every(int);
  if (!ok) throw new Error('The equipment losses reply is malformed.');
  return reply;
}

export function shipLeaves(ships: readonly ShipListing[], scale: number): LossLeaf[] {
  const most = new Map<string, ShipListing>();
  for (const ship of ships) {
    const held = most.get(ship.signature);
    if (!held || ship.count > held.count) most.set(ship.signature, ship);
  }
  const byDesign = new Map<number | null, Map<number, number>>();
  for (const ship of most.values()) {
    let hours = byDesign.get(ship.design);
    if (!hours) { hours = new Map(); byDesign.set(ship.design, hours); }
    hours.set(ship.sinkHour, (hours.get(ship.sinkHour) ?? 0) + ship.count);
  }
  return [...byDesign].sort(([a], [b]) => (a ?? -1) - (b ?? -1)).map(([design, hours]) => {
    let total = 0n;
    const points = [...hours].sort(([a], [b]) => a - b).map(([hour, count]) => {
      total += BigInt(count) * BigInt(scale);
      return { hour, totalRaw: total.toString() };
    });
    return { design, key: `ships:${design ?? 'none'}`, points, exact: [], lowerBoundFrom: null };
  });
}

const filled = new WeakMap<LossesReply, LossesReply>();
export function withShips(reply: LossesReply, _hour?: number): LossesReply {
  let made = filled.get(reply);
  if (!made) {
    made = { ...reply, causes: reply.causes.map(cause => cause.source.kind === 'sunkShips' && cause.unknown === null
      ? { ...cause, leaves: shipLeaves(reply.ships, reply.amountScale) } : cause) };
    filled.set(reply, made);
  }
  return made;
}

export function withHead(reply: LossesReply, hour: number, total: (leaf: LossLeaf) => number | bigint | string | null): LossesReply {
  return { ...reply, causes: reply.causes.map(cause => {
    if (cause.source.kind === 'sunkShips' || cause.unknown !== null) return cause;
    const missing: UnknownSpan[] = [];
    const leaves = cause.leaves.map(leaf => {
      const value = total(leaf);
      if (value === null) return { ...leaf, points: [], exact: [] };
      if (typeof value === 'string') {
        missing.push({ firstHour: hour, lastHour: hour, reason: 'unread', detail: value });
        return {
          ...leaf,
          exact: [...leaf.exact.filter(point => point.hour < hour), { hour, totalRaw: null }],
        };
      }
      let raw: string;
      if (typeof value === 'bigint') {
        raw = value.toString();
      } else if (Number.isSafeInteger(value)) {
        raw = BigInt(value).toString();
      } else {
        const existingExact = exactPointAt(leaf, hour)?.totalRaw ?? null;
        if (existingExact !== null && Number(BigInt(existingExact)) === value) {
          raw = existingExact;
        } else {
          missing.push({ firstHour: hour, lastHour: hour, reason: 'unread', detail: 'Exact 64-bit total unavailable' });
          return {
            ...leaf,
            exact: [...leaf.exact.filter(point => point.hour < hour), { hour, totalRaw: null }],
          };
        }
      }
      return {
        ...leaf,
        points: [...leaf.points.filter(point => point.hour < hour), { hour, totalRaw: raw }],
        exact: [...leaf.exact.filter(point => point.hour < hour), { hour, totalRaw: raw }],
      };
    });
    return { ...cause, leaves, unknownSpans: missing.length ? [...cause.unknownSpans, missing[0]] : cause.unknownSpans };
  }) };
}

export function atHead(reply: LossesReply, hour: number, cells: LiveValues<number>, named: ReadonlySet<string>): LossesReply {
  return withHead(withShips(reply, hour), hour, (leaf: LossLeaf) => {
    if (!named.has(leaf.key)) return null;
    const key = liveKey(leaf.key, 'total');
    return cells.raw?.(key) ?? cells.get(key) ?? cells.missing(key);
  });
}

export function exactPointAt(leaf: LossLeaf, hour: number): ExactLossPoint | null {
  let low = 0;
  let high = leaf.exact.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const found = leaf.exact[middle];
    if (found.hour === hour) return found;
    if (found.hour < hour) low = middle + 1; else high = middle - 1;
  }
  return null;
}

function latestExactAt(leaf: LossLeaf, hour: number): LossPoint | null {
  let low = 0;
  let high = leaf.exact.length - 1;
  let at = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (leaf.exact[middle].hour <= hour) { at = middle; low = middle + 1; } else high = middle - 1;
  }
  while (at >= 0) {
    const found = leaf.exact[at];
    if (found.totalRaw !== null) return { hour: found.hour, totalRaw: found.totalRaw };
    at--;
  }
  return null;
}

export function binPointAt(leaf: LossLeaf, hour: number): LossPoint | null {
  let low = 0;
  let high = leaf.points.length - 1;
  let found: LossPoint | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (leaf.points[middle].hour <= hour) { found = leaf.points[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found;
}

export function pointAt(leaf: LossLeaf, hour: number): LossPoint | null {
  return pointIn(leaf, hour, null);
}

export function pointIn(leaf: LossLeaf, hour: number, minHour: number | null): LossPoint | null {
  const exact = exactPointAt(leaf, hour);
  if (exact) {
    if (exact.totalRaw === null) return null;
    return minHour === null || exact.hour >= minHour ? { hour: exact.hour, totalRaw: exact.totalRaw } : null;
  }
  const rawBin = binPointAt(leaf, hour);
  const bin = rawBin && (minHour === null || rawBin.hour >= minHour) ? rawBin : null;
  const rawPrev = latestExactAt(leaf, hour);
  const prev = rawPrev && (minHour === null || rawPrev.hour >= minHour) ? rawPrev : null;
  if (prev && (!bin || prev.hour >= bin.hour)) return prev;
  return bin;
}

const peaks = new WeakMap<LossLeaf, { firstHour: number; value: number }[]>();
function weightAt(leaf: LossLeaf, hour: number, from?: number | null, minHour: number | null = null): number {
  if (from != null || minHour !== null) {
    const floor = from != null && minHour !== null ? Math.max(from, minHour) : (from ?? minHour!);
    const baseline = from == null ? 0n : BigInt(pointIn(leaf, from - 1, minHour)?.totalRaw ?? '0');
    let peak = leaf.points.reduce((best, point) => point.hour < floor || point.hour > hour ? best
      : Math.max(best, Math.abs(Number(BigInt(point.totalRaw) - baseline))), 0);
    for (const point of leaf.exact) {
      if (point.totalRaw === null || point.hour < floor || point.hour > hour) continue;
      peak = Math.max(peak, Math.abs(Number(BigInt(point.totalRaw) - baseline)));
    }
    return peak;
  }
  let steps = peaks.get(leaf);
  if (!steps) {
    steps = [];
    let peak = 0;
    const all = [...leaf.points, ...leaf.exact.filter((point): point is LossPoint => point.totalRaw !== null)]
      .sort((a, b) => a.hour - b.hour);
    for (const point of all) {
      const value = Math.abs(Number(BigInt(point.totalRaw)));
      if (value > peak) { peak = value; steps.push({ firstHour: point.hour, value }); }
    }
    peaks.set(leaf, steps);
  }
  return entryAt(steps, hour)?.value ?? 0;
}

export const causeName = (cause: LossCause) => contentTitle(cause.name.text ?? cause.name.template, cause.name.key, 'Cause name unavailable');
export const typeName = (design: LossDesign | undefined) => contentTitle(design?.archetypeName, design?.archetype, 'Equipment type name unavailable');

const spansAt = (cause: LossCause, hour: number) => [...new Map(cause.unknownSpans
  .filter(span => span.firstHour <= hour && hour <= span.lastHour)
  .map(span => [JSON.stringify([span.reason, span.detail]), span])).values()];
const destructionReason = (span: UnknownSpan) => span.reason.startsWith('destruction');

export const unknownCauses = (reply: LossesReply, hour: number) => reply.causes.filter(cause => cause.loss).flatMap(cause => {
  if (cause.unknown !== null) return [{ id: sourceKey(cause.source), label: `${causeName(cause)}: Unavailable` }];
  if (!spansAt(cause, hour).length) return [];
  const isDestruction = cause.source.kind === 'destruction';
  const floor = isDestruction ? destructionFloorAt(reply, hour) : null;
  const measured = (!isDestruction || floor !== null) && cause.leaves.some(leaf => pointIn(leaf, hour, floor));
  return [{ id: sourceKey(cause.source), label: `${causeName(cause)}: ${measured ? 'Incomplete' : 'Unavailable'}` }];
});

type LandState = 'logged' | 'partial' | 'notLogged' | 'unknown';
const stateOf = (coverage: Coverage): LandState => typeof coverage === 'object' ? 'partial' : coverage === 'pruneUnobserved' ? 'unknown' : coverage;
export function coverageAt(reply: LossesReply, hour: number): Coverage {
  const span = entryAt(reply.coverage, hour);
  return span && span.lastHour >= hour ? span.coverage : 'unknown';
}

export function coverageHours(reply: LossesReply, first: number, last: number): Record<LandState, number> {
  const counts = { logged: 0, partial: 0, notLogged: 0, unknown: 0 };
  let covered = 0;
  for (const span of reply.coverage) {
    const hours = Math.max(0, Math.min(last, span.lastHour) - Math.max(first, span.firstHour) + 1);
    counts[stateOf(span.coverage)] += hours;
    covered += hours;
  }
  counts.unknown += Math.max(0, last - first + 1 - covered);
  return counts;
}

export interface LossWindow { first: number; hour: number; end: number; campaign: boolean; from?: number | null;
  since?: number | null }

export function lossWindow(window: { firstHour: number; lastHour: number } | null,
  branch: { firstHour: number; lastHour: number }, mode: string): LossWindow | null {
  if (!window || !int(window.lastHour) || window.lastHour < branch.firstHour || window.lastHour > branch.lastHour
    || mode !== 'follow' && (!int(window.firstHour) || window.firstHour < branch.firstHour || window.firstHour > window.lastHour)) return null;
  return { first: mode === 'range' ? window.firstHour : branch.firstHour, hour: window.lastHour,
    end: mode === 'range' ? window.lastHour : branch.lastHour, campaign: mode === 'campaign' || mode === 'range',
    from: mode === 'range' ? window.firstHour : null };
}

export function lossAmount(leaf: LossLeaf, hour: number, from?: number | null, minHour: number | null = null): bigint | null {
  const point = pointIn(leaf, hour, minHour);
  return point ? BigInt(point.totalRaw) - (from == null ? 0n : BigInt(pointIn(leaf, from - 1, minHour)?.totalRaw ?? '0')) : null;
}

export const lossKey = (cause: LossCause, design: LossDesign | undefined) => JSON.stringify([sourceKey(cause.source), design?.variant ?? null]);
export const lossDesignName = (design: LossDesign | undefined, hour: number) => {
  if (!design) return 'Design not recorded';
  const read = entryAt(design.names, hour);
  return (read?.known ? read.name : null) ?? 'Name unavailable';
};

export interface LossItem extends SunburstItem {
  weight?: number;
  incomplete?: boolean;
}
export interface LossTotal extends SunburstTotal { exact: bigint | null; incomplete: boolean }

const NO_DESTRUCTION_READ = 'No destruction drain was recorded at this hour.';
export function destructionRunAt(reply: LossesReply, hour: number): DestructionRun | null {
  const run = entryAt(reply.destructionRuns, hour);
  return run && run.lastHour >= hour ? run : null;
}

function destructionStartFor(reply: LossesReply, hour: number): number | null {
  const run = destructionRunAt(reply, hour);
  if (!run) return null;
  return run.countedFrom ?? run.withheldFrom ?? null;
}

function destructionCountedFor(reply: LossesReply, hour: number): number | null {
  return destructionRunAt(reply, hour)?.countedFrom ?? null;
}

function destructionFloorAt(reply: LossesReply, hour: number): number | null {
  const run = destructionRunAt(reply, hour);
  return run?.countedFrom != null && hour >= run.countedFrom ? Math.max(run.firstHour, run.countedFrom) : null;
}

export function destructionCoverageAt(reply: LossesReply, hour: number): string | null {
  const span = entryAt(reply.destructionCoverage, hour);
  return span && span.lastHour >= hour ? span.reason : NO_DESTRUCTION_READ;
}

export function destructionCoverageHours(reply: LossesReply, first: number, last: number) {
  const run = destructionRunAt(reply, last);
  const start = run ? (run.countedFrom ?? run.withheldFrom ?? null) : null;
  first = Math.max(first, run && start !== null ? Math.max(run.firstHour, start) : last + 1);
  let recorded = 0;
  let unknown = 0;
  const reasons = new Set<string>();
  for (const span of reply.destructionCoverage) {
    const hours = Math.max(0, Math.min(last, span.lastHour) - Math.max(first, span.firstHour) + 1);
    if (!hours) continue;
    if (span.reason === null) recorded += hours;
    else { unknown += hours; reasons.add(span.reason); }
  }
  const missing = Math.max(0, last - first + 1 - recorded - unknown);
  if (missing) { unknown += missing; reasons.add(NO_DESTRUCTION_READ); }
  return { recorded, unknown, reasons: [...reasons] };
}

export function destructionLegend(reply: LossesReply, window: LossWindow, tokens: Pick<ChartTokens, 'positive' | 'surface' | 'text'>): LegendItem[] {
  const first = destructionStartFor(reply, window.hour);
  if (first === null || window.hour < first) return [];
  const missing = window.campaign ? destructionCoverageHours(reply, window.first, window.hour).unknown > 0
    : destructionCoverageAt(reply, window.hour) !== null;
  return [{ id: missing ? 'unknown' : 'recorded', color: missing ? mix(tokens.surface, tokens.text, 0.1) : tokens.positive,
    label: missing ? 'Incomplete' : 'Complete' }];
}

export function coverageLegend(reply: LossesReply, window: LossWindow, tokens: Pick<ChartTokens, 'positive' | 'warning' | 'error' | 'surface' | 'text'>): LegendItem[] {
  if (reply.landSource === 'destruction') return destructionLegend(reply, window, tokens);
  const counts = coverageHours(reply, window.first, window.hour);
  const incomplete = window.campaign ? counts.partial + counts.notLogged + counts.unknown > 0 : stateOf(coverageAt(reply, window.hour)) !== 'logged';
  return [{ id: incomplete ? 'partial' : 'logged', color: incomplete ? tokens.warning : tokens.positive,
    label: incomplete ? 'Incomplete' : 'Complete' }];
}

export function landNote(reply: LossesReply, first: number, hour: number): string | undefined {
  if (reply.landSource === 'destruction') return reply.landLowerBound ? 'Incomplete total.' : undefined;
  const counts = coverageHours(reply, first, hour);
  return counts.partial || counts.notLogged || counts.unknown ? 'Incomplete total.' : undefined;
}

export function lossItems(reply: LossesReply, window: LossWindow): LossItem[] {
  const items: LossItem[] = [];
  const land = landNote(reply, window.first, window.hour);
  const destructionFloor = destructionFloorAt(reply, window.hour);
  const endDestructionFloor = destructionFloorAt(reply, window.end);
  for (const cause of reply.causes) {
    if (!cause.loss) continue;
    const causePath = [{ id: `cause:${sourceKey(cause.source)}`, label: causeName(cause) }];
    if (cause.unknown !== null) {
      items.push({ path: causePath, value: null, final: null, finalUnknown: true, unknownValue: true, incomplete: true, note: 'Unavailable' });
      continue;
    }
    const spans = spansAt(cause, window.hour), finalSpans = spansAt(cause, window.end);
    const destructionMissing = spans.some(destructionReason);
    const unread = spans.some(span => span.reason === 'unread');
    const ended = spans.some(span => span.reason === 'endedAfterLastRead');
    const isDestruction = cause.source.kind === 'destruction';
    const started = !isDestruction || destructionFloor !== null;
    const endStarted = !isDestruction || endDestructionFloor !== null;
    const minHour = isDestruction ? destructionFloor : null;
    const endMinHour = isDestruction ? endDestructionFloor : null;
    const incomplete = unread || ended || destructionMissing || !!(cause.domain === 'land' && started && land);
    const finalUnknown = finalSpans.length > 0 || !!(cause.domain === 'land' && landNote(reply, window.first, window.end));
    const noAmount = !started || !cause.leaves.some(leaf => pointIn(leaf, window.hour, minHour));
    const note = incomplete ? noAmount ? 'Unavailable' : 'Incomplete total.' : undefined;
    const noKnownAmount = noAmount && incomplete;
    items.push({ path: causePath, value: noKnownAmount ? null : 0, exact: 0n, final: 0,
      unknownValue: noKnownAmount, incomplete, finalUnknown, note: note || undefined, present: started || spans.length > 0 });
    for (const leaf of cause.leaves) {
      const point = started ? pointIn(leaf, window.hour, minHour) : null;
      const design = leaf.design === null ? undefined : reply.designs[leaf.design];
      const exact = started ? lossAmount(leaf, window.hour, window.from, minHour) : null;
      const final = endStarted ? lossAmount(leaf, window.end, window.from, endMinHour) : null;
      const path = [...causePath, { id: `type:${design?.archetype ?? '?unknown'}`, label: typeName(design) },
        { id: `design:${design?.variant ?? 'none'}`, label: lossDesignName(design, window.hour), keepZero: true }];
      items.push({ path, value: exact === null ? null : Number(exact), exact, final: final === null ? null : Number(final),
        weight: endStarted ? weightAt(leaf, window.end, window.from, endMinHour) : 0, present: point !== null,
        incomplete: unread || isDestruction && incomplete || !!(cause.domain === 'land' && land)
          || leaf.lowerBoundFrom !== null && leaf.lowerBoundFrom <= window.hour,
        finalUnknown: finalSpans.some(span => span.reason === 'unread') || isDestruction && finalUnknown
          || !!(cause.domain === 'land' && landNote(reply, window.first, window.end))
          || leaf.lowerBoundFrom !== null && leaf.lowerBoundFrom <= window.end,
        note: !ended || unread || isDestruction || !!land
          || leaf.lowerBoundFrom !== null && leaf.lowerBoundFrom <= window.hour ? note || undefined : undefined,
        counted: [lossKey(cause, design)] });
    }
  }
  return items;
}

export function formatLosses(value: number, exact: bigint | null, scale: number): string {
  const raw = exact ?? BigInt(Math.round(value));
  const amount = formatRate(raw, Math.round(Math.log10(scale)));
  return raw < 0n ? `Returned: ${amount}` : amount;
}

export function lossesFootnote(reply: LossesReply, window: LossWindow): string {
  return [landNote(reply, window.first, window.hour), destructionCountingNote(reply, window.hour),
    reply.landSource === 'combatLog' && reply.retainedMonths !== 0 ? 'Includes earlier Combat Log losses.' : undefined].filter(Boolean).join(' ');
}

export function lossesTotal(items: readonly LossItem[]): LossTotal {
  let value = 0;
  let exact = 0n;
  let final = 0;
  let incomplete = false;
  let finalUnknown = false;
  let measured = false;
  for (const item of items) {
    final += item.final ?? 0;
    finalUnknown ||= item.finalUnknown === true || item.final === null;
    if (item.present === false) continue;
    measured ||= item.value !== null;
    value += item.value ?? 0;
    exact += item.exact ?? 0n;
    incomplete ||= item.incomplete === true || item.value === null;
  }
  return { value: measured ? value : null, exact: measured ? exact : null, final, finalUnknown, incomplete, unknownValue: !measured,
    note: incomplete ? 'Incomplete total.' : undefined };
}

export function destructionCountingNote(reply: LossesReply, hour: number): string {
  const countedFrom = destructionCountedFor(reply, hour);
  return countedFrom === null || hour < countedFrom ? 'Unavailable' : '';
}

export function stepHours(hours: readonly number[], key: NavigationKey, origin: number | null, first: number, last: number): number | null {
  const within = hours.filter(hour => hour >= first && hour <= last);
  const after = key === 'Home' || key === 'ArrowRight' && origin === null ? within
    : key === 'ArrowRight' ? within.filter(hour => hour > origin!) : [];
  const before = key === 'End' || key === 'ArrowLeft' && origin === null ? within
    : key === 'ArrowLeft' ? within.filter(hour => hour < origin!) : [];
  if (key === 'Home' || key === 'ArrowRight') return after.length ? Math.min(...after) : null;
  return before.length ? Math.max(...before) : null;
}

export function withEdges(key: NavigationKey, answer: number | null, origin: number | null, first: number): number | null {
  if (key === 'Home') return first;
  if (key === 'End') return answer ?? first;
  if (key === 'ArrowLeft') return answer ?? (origin !== null && origin > first ? first : null);
  return answer ?? origin;
}

export function nearest(key: NavigationKey, a: number | null, b: number | null): number | null {
  if (a === null || b === null) return a ?? b;
  return key === 'Home' || key === 'ArrowRight' ? Math.min(a, b) : Math.max(a, b);
}

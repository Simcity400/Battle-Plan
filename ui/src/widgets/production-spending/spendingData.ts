import type { ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun, ProductionSpendingUnknown } from '../../generated/dashboard';
import type { SunburstItem, SunburstTotal } from '../../charts/sunburst/layout';
import { runAt, sameStretch } from '../../charts/series.ts';
import { formatIc } from '../../dashboard/icDecimals.ts';
import { DOMAINS, UNRESOLVED_ARM, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import { readAt } from '../../dashboard/reads.ts';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import type { FactoryKind } from './definition.ts';

export const LEVELS = ['domainLevel', 'typeLevel', 'variantLevel'] as const;
/** The centre's unit. */
export const UNIT = 'IC';
const ATTRIBUTION_NOTE = 'Equipment type unknown.';

export interface PreparedProductionSpendingReply extends ProductionSpendingReply {
  continuity?: Record<string, readonly [boolean, boolean]>;
  peaks?: Record<string, string>;
  exactHours?: readonly number[];
  /** Before preparation completes: the prepared prefix the reply answers ([`preparedPrefix`]). */
  coverage?: unknown;
  pending?: unknown;
}

export const UNKNOWN_LABELS: Readonly<Record<ProductionSpendingUnknown, string>> = {
  notRecorded: 'Spending not recorded.',
  spendingUnavailable: 'Spending unavailable at this time.',
  lostOutput: 'Final total missing.',
  missedPublication: 'Spending unavailable at this time.',
  recordingGap: 'Hours were not recorded',
  observationRestarted: 'Campaign total unavailable.',
  observationMismatch: 'Production totals start on different dates.',
  countryChanged: 'Country changed during this period.',
  rocketTableUnavailable: 'Rocket spending not recorded.',
  rocketInputInexact: 'Rocket spending total unknown.',
  rocketOwnerUnreadable: 'Rocket producer unknown.',
  rocketDesignUnreadable: 'Rocket design unknown.',
  preparing: 'Still being prepared.',
};

type UnknownReason = ProductionSpendingUnknown | 'outsideRead' | 'observationChanged' | 'unattributed';
export const unknownLabel = (reason: string) => UNKNOWN_LABELS[reason as ProductionSpendingUnknown]
  ?? (reason === 'observationChanged' ? 'Campaign total unavailable.' : reason === 'unattributed' ? ATTRIBUTION_NOTE : 'Spending unavailable at this time.');

/** A design whose domain is unresolved could be either, so both charts keep it. */
const unattributed = (leaf: ProductionSpendingLeaf) => !leaf.rocket && leaf.domain == null;
export const inFactoryKind = (leaf: ProductionSpendingLeaf, kind: FactoryKind) =>
  unattributed(leaf) || (kind === 'dockyards' ? !leaf.rocket && leaf.domain === 'naval' : leaf.rocket || leaf.domain !== 'naval');

const RAW = /^-?\d{1,39}$/;
/** What a reply split by purpose keys its leaves by. */
const PURPOSES: readonly unknown[] = ['new', 'refit', 'converted', 'unknown'];
const int = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const hours = (value: unknown): value is number[] => Array.isArray(value) && value.every((hour, index) => int(hour)
  && (index === 0 || value[index - 1] < hour));
const runs = (value: unknown): value is ProductionSpendingRun[] => Array.isArray(value) && value.every((run, index) => typeof run === 'object' && run != null
  && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour && (index === 0 || value[index - 1].lastHour < run.firstHour)
  && (run.totalRaw === null ? Object.hasOwn(UNKNOWN_LABELS, run.reason) : typeof run.totalRaw === 'string' && RAW.test(run.totalRaw) && run.reason === null));

/** Fixed-point totals and missing values keep their distinct meanings. */
export function validateProductionSpendingReply(data: unknown): ProductionSpendingReply {
  const reply = data as PreparedProductionSpendingReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && reply.decimals === 5 && hours(reply.reads) && hours(reply.observations) && runs(reply.total) && runs(reply.equipmentTotal) && runs(reply.rocketTotal)
    && hours(reply.equipmentReads) && hours(reply.rocketReads) && hours(reply.equipmentObservations) && hours(reply.rocketObservations)
    && text(reply.rocketKey) && text(reply.rocketName)
    && Array.isArray(reply.production) && reply.production.every((run, index) => run != null
      && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour
      && (run.present === null || typeof run.present === 'boolean')
      && (run.emptyCensusHour === null || int(run.emptyCensusHour) && run.emptyCensusHour <= run.firstHour
        && readAt(reply.equipmentReads, run.emptyCensusHour) === run.emptyCensusHour && run.present === false)
      && (index === 0 || reply.production[index - 1].lastHour < run.firstHour))
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.leaves) && reply.leaves.every(leaf => typeof leaf === 'object' && leaf != null
      && typeof leaf.variant === 'string' && leaf.variant.length > 0 && typeof leaf.typeKey === 'string' && leaf.typeKey.length > 0 && text(leaf.name)
      && (leaf.domain === null || DOMAINS.includes(leaf.domain)) && typeof leaf.rocket === 'boolean' && runs(leaf.runs)
      && (leaf.purpose === null || PURPOSES.includes(leaf.purpose)) && runs(leaf.unproven)
      && Array.isArray(leaf.names) && leaf.names.every((name, index) => typeof name === 'object' && name != null && int(name.firstHour)
        && text(name.name) && text(name.design) && typeof name.known === 'boolean' && (index === 0 || leaf.names[index - 1].firstHour < name.firstHour)))
    && new Set(reply.leaves.map(leafKey)).size === reply.leaves.length
    && (reply.continuity === undefined || reply.continuity !== null && typeof reply.continuity === 'object'
      && Object.values(reply.continuity).every(pair => Array.isArray(pair) && pair.length === 2 && pair.every(value => typeof value === 'boolean')))
    && (reply.peaks === undefined || reply.peaks !== null && typeof reply.peaks === 'object'
      && Object.values(reply.peaks).every(value => typeof value === 'string' && RAW.test(value)))
    && (reply.exactHours === undefined || hours(reply.exactHours));
  if (!ok) throw new Error('The production spending reply is malformed.');
  preparedPrefix(reply);
  return reply;
}

/** A leaf's identity: its design and source, and its purpose in a reply split by purpose. */
export const leafKey = (leaf: ProductionSpendingLeaf) => JSON.stringify([leaf.variant, leaf.rocket, leaf.purpose]);

export interface SpendingAmount { exact: bigint | null; reason: UnknownReason | null }

/** No amount outside a run: a missing hour never holds the preceding value. */
export function spendingAt(values: readonly ProductionSpendingRun[], hour: number): SpendingAmount {
  const run = runAt(values, Math.floor(hour));
  return run ? { exact: run.totalRaw == null ? null : BigInt(run.totalRaw), reason: run.reason }
    : { exact: null, reason: 'outsideRead' };
}

export interface SpendingWindow { from: number | null; hour: number; end: number }

/** The dates a reply answers for `window`: the requested ones, or, before
 * preparation reaches them, its prepared prefix's last hour. Applied IC is a
 * cumulative endpoint, so the prefix's totals are those spent through that
 * hour and the later hours are still being prepared. Null when the prefix
 * has not reached a range's start. */
export function preparedWindow(reply: ProductionSpendingReply, window: SpendingWindow): SpendingWindow | null {
  const prefix = preparedPrefix(reply);
  if (!prefix?.pending.length) return window;
  const last = prefix.coverage.lastHour;
  if (window.from != null && window.from > last) return null;
  return { from: window.from, hour: Math.min(window.hour, last), end: Math.min(window.end, last) };
}

export const noProductionAt = (reply: ProductionSpendingReply, hour: number) => {
  const evidence = runAt(reply.production, Math.floor(hour));
  return evidence?.present === false && evidence.emptyCensusHour != null && spendingAt(reply.total, hour).exact === 0n;
};

export function spendingEmptyLabel(reply: ProductionSpendingReply, hour: number, from: number | null = null, kind?: FactoryKind): string {
  const amount = kindAmount(reply, from, hour, kind);
  if (amount.exact == null) return unknownLabel(amount.reason ?? 'outsideRead');
  if (noProductionAt(reply, hour)) return 'No military production yet.';
  const factories = kind === 'dockyards' ? 'dockyard' : kind === 'military' ? 'military factory' : 'factory';
  return runAt(reply.production, Math.floor(hour))?.present == null
    ? `No ${factories} IC spending recorded.` : `No ${factories} IC spent in this view.`;
}

/** The amount `values` reached at `hour`, or spent since `from` within one
 * uninterrupted observation; unknown otherwise. */
export function amountIn(reply: ProductionSpendingReply, values: readonly ProductionSpendingRun[], from: number | null, hour: number,
  observations: readonly number[] = reply.observations, source: 'equipment' | 'rocket' | 'combined' = 'combined'): SpendingAmount {
  const amount = spendingAt(values, hour);
  if (from == null || amount.exact == null) return amount;
  const observation = readAt(observations, hour);
  if (observation != null && from < observation) return { exact: null, reason: 'observationChanged' };
  if (from === observation) return amount;
  const before = spendingAt(values, from - 1);
  if (before.exact == null) return before;
  const covered = (reply as PreparedProductionSpendingReply).continuity?.[hour];
  if (covered ? source === 'combined' ? !covered.every(Boolean) : !covered[source === 'rocket' ? 1 : 0]
    : !sameStretch(values, from - 1, hour)) return { exact: null, reason: 'outsideRead' };
  const exact = amount.exact - before.exact;
  return { exact, reason: null };
}

export interface SpendingFinal { exact: bigint | null; geometry: number }

/** Prepared once per date window. An unknown endpoint's earlier maximum is only a
 * stable colour/order key, never a claimed whole total. */
export function spendingFinals(reply: ProductionSpendingReply, from: number | null, end: number, kind?: FactoryKind): SpendingFinal[] {
  return shownLeaves(reply, kind).map(leaf => {
    const observations = leaf.rocket ? reply.rocketObservations : reply.equipmentObservations;
    const amount = amountIn(reply, leaf.runs, from, end, observations, leaf.rocket ? 'rocket' : 'equipment');
    if (amount.exact != null) return { exact: amount.exact, geometry: Number(amount.exact) };
    let maximum = 0n;
    const baseline = from == null || observations.includes(from) ? 0n : spendingAt(leaf.runs, from - 1).exact;
    // A reply split by purpose has new production's peaks only.
    const peak = leaf.purpose != null && leaf.purpose !== 'new' ? undefined : (reply as PreparedProductionSpendingReply).peaks?.[JSON.stringify([leaf.variant, leaf.rocket])];
    if (peak !== undefined) {
      const candidate = baseline === null ? 0n : BigInt(peak) - baseline;
      return { exact: null, geometry: Number(candidate > 0n ? candidate : 0n) };
    }
    for (const run of leaf.runs) {
      if (run.firstHour > end) break;
      if (baseline == null || run.totalRaw == null || (from != null && run.lastHour < from)) continue;
      const candidate = BigInt(run.totalRaw) - baseline;
      if (candidate > maximum) maximum = candidate;
    }
    return { exact: null, geometry: Number(maximum) };
  });
}

/** The shared sunburst owns grouping, navigation, colours and missing totals. */
export function productionSpendingItems(reply: ProductionSpendingReply, window: SpendingWindow,
  finals = spendingFinals(reply, window.from, window.end), kind?: FactoryKind): SunburstItem[] {
  const leaves = shownLeaves(reply, kind);
  const amounts = leaves.map(leaf => amountIn(reply, leaf.runs, window.from, window.hour,
    leaf.rocket ? reply.rocketObservations : reply.equipmentObservations, leaf.rocket ? 'rocket' : 'equipment'));
  const unresolved = leaves.some((leaf, index) => !leaf.rocket && leaf.domain == null && amounts[index].exact !== 0n);
  const unresolvedAtEnd = leaves.some((leaf, index) => !leaf.rocket && leaf.domain == null && finals[index].exact !== 0n);
  const unresolvedRocket = leaves.some((leaf, index) => leaf.rocket && leaf.domain == null && amounts[index].exact !== 0n);
  const unresolvedRocketAtEnd = leaves.some((leaf, index) => leaf.rocket && leaf.domain == null && finals[index].exact !== 0n);
  const items: SunburstItem[] = leaves.map((leaf, index) => {
    const amount = amounts[index];
    const final = finals[index];
    const observed = amount.exact == null || amount.exact !== 0n;
    const attribution = !leaf.rocket && unresolved && leaf.domain != null;
    const finalAttribution = !leaf.rocket && unresolvedAtEnd && leaf.domain != null;
    const typeAttribution = leaf.rocket ? unresolvedRocket && leaf.domain != null : attribution;
    const identity = leaf.rocket ? `rocket:${leaf.variant}` : leaf.variant;
    return {
      path: [
        leaf.rocket ? { id: 'rocket', label: reply.rocketName ?? reply.rocketKey ?? 'Rocket sites' }
          : { id: leaf.domain ?? UNRESOLVED_ARM, label: domainName(leaf.domain ?? UNRESOLVED_ARM) },
        { id: leaf.typeKey, label: leaf.name ?? unresolvedTypeName(leaf.typeKey) },
        { id: identity, label: variantNameAt(leaf.names, window.hour, leaf.variant) },
      ],
      value: amount.exact == null ? null : Number(amount.exact), exact: amount.exact,
      unknownValue: amount.exact == null, unknownLevels: [attribution, attribution, false],
      localUnknownLevels: [false, leaf.rocket && typeAttribution, false],
      note: amount.reason ? unknownLabel(amount.reason) : typeAttribution ? ATTRIBUTION_NOTE : undefined,
      final: final.geometry, finalUnknown: final.exact == null,
      finalUnknownLevels: [finalAttribution, finalAttribution, false],
      localFinalUnknownLevels: [false, leaf.rocket && unresolvedRocketAtEnd && leaf.domain != null, false],
      present: observed || typeAttribution, counted: observed ? [identity] : [],
    };
  });
  const total = kindAmount(reply, window.from, window.hour, kind);
  const finalTotal = kindAmount(reply, window.from, window.end, kind);
  const hasParts = items.some(item => item.present);
  if (total.exact == null || !hasParts || finalTotal.exact == null) items.push({
    path: [], value: total.exact == null ? null : hasParts ? 0 : Number(total.exact), exact: total.exact == null ? null : hasParts ? 0n : total.exact,
    unknownValue: total.exact == null, note: total.reason ? unknownLabel(total.reason) : undefined,
    final: 0, finalUnknown: finalTotal.exact == null,
  });
  return items;
}

const shownLeaves = (reply: ProductionSpendingReply, kind?: FactoryKind) =>
  kind ? reply.leaves.filter(leaf => inFactoryKind(leaf, kind)) : reply.leaves;

/** The country's total, or one kind's: the sum of its designs, unknown while any of
 * them is, or while spending on a design of unresolved domain could belong to either.
 * Dockyards build no Rockets, so only the equipment source's coverage gates theirs. */
function kindAmount(reply: ProductionSpendingReply, from: number | null, hour: number, kind?: FactoryKind): SpendingAmount {
  const all = kind === 'dockyards' ? amountIn(reply, reply.equipmentTotal, from, hour, reply.equipmentObservations, 'equipment')
    : amountIn(reply, reply.total, from, hour);
  if (!kind || all.exact == null) return all;
  let exact = 0n;
  for (const leaf of shownLeaves(reply, kind)) {
    const amount = amountIn(reply, leaf.runs, from, hour, leaf.rocket ? reply.rocketObservations : reply.equipmentObservations, leaf.rocket ? 'rocket' : 'equipment');
    if (amount.exact == null) return amount;
    if (amount.exact !== 0n && unattributed(leaf)) return { exact: null, reason: 'unattributed' };
    exact += amount.exact;
  }
  return { exact, reason: null };
}

/** Country totals do not depend on whether content can attribute every design; a
 * kind's total does, as it is the sum of that kind's designs. */
export function spendingTotal(reply: ProductionSpendingReply, window: SpendingWindow, kind?: FactoryKind): SunburstTotal {
  const amount = kindAmount(reply, window.from, window.hour, kind);
  const final = kindAmount(reply, window.from, window.end, kind);
  return {
    value: amount.exact == null ? null : Number(amount.exact), exact: amount.exact,
    unknownValue: amount.exact == null, note: amount.reason ? unknownLabel(amount.reason) : undefined,
    final: final.exact == null ? 0 : Number(final.exact), finalUnknown: final.exact == null,
  };
}

/** Applied IC (`decimals` recorded places) written to `digits` places, from the
 * exact amount, including values beyond 2^53. */
export const formatAppliedIc = (value: number, exact: bigint | null, decimals: number, digits: number) =>
  formatIc(value, exact, decimals, digits);

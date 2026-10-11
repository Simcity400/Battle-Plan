import type { ProductionSpendingReply, ProductionSpendingRun, ProductionSpendingUnknown } from '../../generated/dashboard';
import type { SunburstItem, SunburstTotal } from '../../charts/sunburst/layout';
import { runAt, sameStretch } from '../../charts/series.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { DOMAINS, UNRESOLVED_ARM, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import { readAt, readNote } from '../../dashboard/reads.ts';

export const LEVELS = ['domainLevel', 'typeLevel', 'variantLevel'] as const;
/** The centre's unit; the value label and footnote say how it is valued. */
export const UNIT = 'IC';
const ATTRIBUTION_NOTE = 'A recorded equipment type is unresolved; its applied IC may belong to this arm or type.';

export const UNKNOWN_LABELS: Readonly<Record<ProductionSpendingUnknown, string>> = {
  notRecorded: 'Exact applied IC was not recorded',
  spendingUnavailable: 'An applied-input read, instant classification or spending total was unavailable',
  lostOutput: 'A line disappeared without its final applied IC total',
  missedPublication: 'A production-output read was missed',
  recordingGap: 'Hours were not recorded',
  observationRestarted: 'Applied IC observation restarted; the whole-campaign total cannot be recovered',
  observationMismatch: 'The production sources count from different dates',
  countryChanged: 'The country holding this tag changed',
  rocketTableUnavailable: 'Independent Rocket production coverage was not recorded or readable',
  rocketInputInexact: 'A Rocket applied input or cumulative total was unavailable',
  rocketOwnerUnreadable: 'A Rocket event has no known producing country',
  rocketDesignUnreadable: 'A Rocket event has no known design',
};

type UnknownReason = ProductionSpendingUnknown | 'outsideRead' | 'observationChanged';
export const unknownLabel = (reason: string) => UNKNOWN_LABELS[reason as ProductionSpendingUnknown]
  ?? (reason === 'observationChanged' ? 'The range crosses a restart of applied IC observation' : 'No production-output read covers this hour');

const RAW = /^-?\d{1,39}$/;
const int = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const hours = (value: unknown): value is number[] => Array.isArray(value) && value.every((hour, index) => int(hour)
  && (index === 0 || value[index - 1] < hour));
const runs = (value: unknown): value is ProductionSpendingRun[] => Array.isArray(value) && value.every((run, index) => typeof run === 'object' && run != null
  && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour && (index === 0 || value[index - 1].lastHour < run.firstHour)
  && (run.totalRaw === null ? Object.hasOwn(UNKNOWN_LABELS, run.reason) : typeof run.totalRaw === 'string' && RAW.test(run.totalRaw) && run.reason === null));

/** Fixed-point totals and missing values keep their distinct meanings. */
export function validateProductionSpendingReply(data: unknown): ProductionSpendingReply {
  const reply = data as ProductionSpendingReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && reply.decimals === 5 && hours(reply.reads) && hours(reply.observations) && runs(reply.total) && runs(reply.rocketTotal)
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
      && Array.isArray(leaf.names) && leaf.names.every((name, index) => typeof name === 'object' && name != null && int(name.firstHour)
        && text(name.name) && text(name.design) && typeof name.known === 'boolean' && (index === 0 || leaf.names[index - 1].firstHour < name.firstHour)))
    && new Set(reply.leaves.map(leaf => JSON.stringify([leaf.variant, leaf.rocket]))).size === reply.leaves.length;
  if (!ok) throw new Error('The production spending reply is malformed.');
  return reply;
}

export interface SpendingAmount { exact: bigint | null; reason: UnknownReason | null }

/** No amount outside a run: a missing hour never holds the preceding value. */
export function spendingAt(values: readonly ProductionSpendingRun[], hour: number): SpendingAmount {
  const run = runAt(values, Math.floor(hour));
  return run ? { exact: run.totalRaw == null ? null : BigInt(run.totalRaw), reason: run.reason }
    : { exact: null, reason: 'outsideRead' };
}

export interface SpendingWindow { from: number | null; hour: number; end: number }

export const noProductionAt = (reply: ProductionSpendingReply, hour: number) => {
  const evidence = runAt(reply.production, Math.floor(hour));
  return evidence?.present === false && evidence.emptyCensusHour != null && spendingAt(reply.total, hour).exact === 0n;
};

export function spendingEmptyLabel(reply: ProductionSpendingReply, hour: number, from: number | null = null): string {
  const amount = amountIn(reply, reply.total, from, hour);
  if (amount.exact == null) return unknownLabel(amount.reason ?? 'outsideRead');
  if (noProductionAt(reply, hour)) return 'No military production yet.';
  return runAt(reply.production, Math.floor(hour))?.present == null
    ? 'No factory IC spent; production presence is not known.' : 'No factory IC spent in this view.';
}

function amountIn(reply: ProductionSpendingReply, values: readonly ProductionSpendingRun[], from: number | null, hour: number,
  observations: readonly number[] = reply.observations): SpendingAmount {
  const amount = spendingAt(values, hour);
  if (from == null || amount.exact == null) return amount;
  const observation = readAt(observations, hour);
  if (observation != null && from < observation) return { exact: null, reason: 'observationChanged' };
  if (from === observation) return amount;
  const before = spendingAt(values, from - 1);
  if (before.exact == null) return before;
  if (!sameStretch(values, from - 1, hour)) return { exact: null, reason: 'outsideRead' };
  const exact = amount.exact - before.exact;
  return { exact, reason: null };
}

export interface SpendingFinal { exact: bigint | null; geometry: number }

/** Prepared once per date window. An unknown endpoint's earlier maximum is only a
 * stable colour/order key, never a claimed whole total. */
export function spendingFinals(reply: ProductionSpendingReply, from: number | null, end: number): SpendingFinal[] {
  return reply.leaves.map(leaf => {
    const observations = leaf.rocket ? reply.rocketObservations : reply.equipmentObservations;
    const amount = amountIn(reply, leaf.runs, from, end, observations);
    if (amount.exact != null) return { exact: amount.exact, geometry: Number(amount.exact) };
    let maximum = 0n;
    const baseline = from == null || observations.includes(from) ? 0n : spendingAt(leaf.runs, from - 1).exact;
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
  finals = spendingFinals(reply, window.from, window.end)): SunburstItem[] {
  const amounts = reply.leaves.map(leaf => amountIn(reply, leaf.runs, window.from, window.hour,
    leaf.rocket ? reply.rocketObservations : reply.equipmentObservations));
  const unresolved = reply.leaves.some((leaf, index) => !leaf.rocket && leaf.domain == null && amounts[index].exact !== 0n);
  const unresolvedAtEnd = reply.leaves.some((leaf, index) => !leaf.rocket && leaf.domain == null && finals[index].exact !== 0n);
  const unresolvedRocket = reply.leaves.some((leaf, index) => leaf.rocket && leaf.domain == null && amounts[index].exact !== 0n);
  const unresolvedRocketAtEnd = reply.leaves.some((leaf, index) => leaf.rocket && leaf.domain == null && finals[index].exact !== 0n);
  const items: SunburstItem[] = reply.leaves.map((leaf, index) => {
    const amount = amounts[index];
    const final = finals[index];
    const observed = amount.exact == null || amount.exact !== 0n;
    const attribution = !leaf.rocket && unresolved && leaf.domain != null;
    const finalAttribution = !leaf.rocket && unresolvedAtEnd && leaf.domain != null;
    const typeAttribution = leaf.rocket ? unresolvedRocket && leaf.domain != null : attribution;
    const identity = leaf.rocket ? `rocket:${leaf.variant}` : leaf.variant;
    return {
      path: [
        leaf.rocket ? { id: 'rocket', label: reply.rocketName ?? reply.rocketKey ?? 'Rocket sites (name unavailable)' }
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
  const total = amountIn(reply, reply.total, window.from, window.hour);
  const finalTotal = amountIn(reply, reply.total, window.from, window.end);
  const hasParts = items.some(item => item.present);
  if (total.exact == null || !hasParts || finalTotal.exact == null) items.push({
    path: [], value: total.exact == null ? null : hasParts ? 0 : Number(total.exact), exact: total.exact == null ? null : hasParts ? 0n : total.exact,
    unknownValue: total.exact == null, note: total.reason ? unknownLabel(total.reason) : undefined,
    final: 0, finalUnknown: finalTotal.exact == null,
  });
  return items;
}

/** Country totals do not depend on whether content can attribute every design. */
export function spendingTotal(reply: ProductionSpendingReply, window: SpendingWindow): SunburstTotal {
  const amount = amountIn(reply, reply.total, window.from, window.hour);
  const final = amountIn(reply, reply.total, window.from, window.end);
  return {
    value: amount.exact == null ? null : Number(amount.exact), exact: amount.exact,
    unknownValue: amount.exact == null, note: amount.reason ? unknownLabel(amount.reason) : undefined,
    final: final.exact == null ? 0 : Number(final.exact), finalUnknown: final.exact == null,
  };
}

/** Every recorded fixed5 digit is retained, including values beyond 2^53. */
export const formatAppliedIc = (value: number, exact: bigint | null, decimals = 5) =>
  formatAmount(exact ?? BigInt(Math.round(value)), decimals, decimals);

export function spendingFootnote(reply: ProductionSpendingReply, window: SpendingWindow, growth: 'share' | 'fill' = 'share'): string {
  const since = [['Equipment lines', reply.equipmentObservations[0]], ['Rocket sites', reply.rocketObservations[0]]] as const;
  const counting = since.flatMap(([source, hour]) => hour == null ? [] : [`${source} counting began ${formatDateTime(hour)}.`]).join(' ');
  const read = [readNote(readAt(reply.equipmentReads, window.hour), window.hour, 'equipment output is read once a day.'),
    readNote(readAt(reply.rocketReads, window.hour), window.hour, 'Rocket output is read once a day.')].filter(Boolean).join(' ');
  const total = amountIn(reply, reply.total, window.from, window.hour);
  const why = total.exact == null ? ` Total not known: ${unknownLabel(total.reason ?? 'outsideRead')}.` : '';
  const finalUnknown = amountIn(reply, reply.total, window.from, window.end).exact == null;
  return `Exact signed factory IC applied to equipment-line and Rocket-site progress, including ticks without delivery. Excludes instant progress and IC before counting began.${counting ? ` ${counting}` : ''} Gaps and restarts leave whole-campaign totals not known.${why}${read ? ` ${read}` : ''}${finalUnknown && growth === 'fill' ? ' Fill uses Share while the whole total is not known.' : ''}`;
}

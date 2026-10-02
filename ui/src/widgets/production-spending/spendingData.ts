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
const ATTRIBUTION_NOTE = 'A recorded equipment type is unresolved; its deliveries may belong to this arm or type.';

export const UNKNOWN_LABELS: Readonly<Record<ProductionSpendingUnknown, string>> = {
  missingCost: 'The supplying read did not record the line’s cost',
  invalidCost: 'The supplying read’s line cost is not valid',
  outputInexact: 'The collector could not count every delivery',
  lostOutput: 'A line disappeared without its final delivery count',
  missedPublication: 'A production-output read was missed',
  recordingGap: 'Hours were not recorded',
  observationRestarted: 'Delivery counting restarted; the whole-campaign total cannot be recovered',
  countryChanged: 'The country holding this tag changed',
};

type UnknownReason = ProductionSpendingUnknown | 'outsideRead' | 'observationChanged';
export const unknownLabel = (reason: string) => UNKNOWN_LABELS[reason as ProductionSpendingUnknown]
  ?? (reason === 'observationChanged' ? 'The range crosses a restart of delivery counting' : 'No production-output read covers this hour');

const RAW = /^\d{1,39}$/;
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
    && reply.decimals === 5 && hours(reply.reads) && hours(reply.observations) && runs(reply.total)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.leaves) && reply.leaves.every(leaf => typeof leaf === 'object' && leaf != null
      && typeof leaf.variant === 'string' && leaf.variant.length > 0 && typeof leaf.typeKey === 'string' && leaf.typeKey.length > 0 && text(leaf.name)
      && (leaf.domain === null || DOMAINS.includes(leaf.domain)) && runs(leaf.runs)
      && Array.isArray(leaf.names) && leaf.names.every((name, index) => typeof name === 'object' && name != null && int(name.firstHour)
        && text(name.name) && text(name.design) && typeof name.known === 'boolean' && (index === 0 || leaf.names[index - 1].firstHour < name.firstHour)))
    && new Set(reply.leaves.map(leaf => JSON.stringify([leaf.domain, leaf.typeKey, leaf.variant]))).size === reply.leaves.length;
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

function amountIn(reply: ProductionSpendingReply, values: readonly ProductionSpendingRun[], from: number | null, hour: number): SpendingAmount {
  const amount = spendingAt(values, hour);
  if (from == null || amount.exact == null) return amount;
  const observation = readAt(reply.observations, hour);
  if (observation == null || from < observation) return { exact: null, reason: 'observationChanged' };
  if (from === observation) return amount;
  const before = spendingAt(values, from - 1);
  if (before.exact == null) return before;
  if (!sameStretch(values, from - 1, hour)) return { exact: null, reason: 'outsideRead' };
  const exact = amount.exact - before.exact;
  return exact < 0n ? { exact: null, reason: 'observationChanged' } : { exact, reason: null };
}

export interface SpendingFinal { exact: bigint | null; geometry: number }

/** Prepared once per date window. An unknown endpoint's earlier maximum is only a
 * stable colour/order key, never a claimed whole total. */
export function spendingFinals(reply: ProductionSpendingReply, from: number | null, end: number): SpendingFinal[] {
  return reply.leaves.map(leaf => {
    const amount = amountIn(reply, leaf.runs, from, end);
    if (amount.exact != null) return { exact: amount.exact, geometry: Number(amount.exact) };
    let maximum = 0n;
    const baseline = from == null || reply.observations.includes(from) ? 0n : spendingAt(leaf.runs, from - 1).exact;
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
  const amounts = reply.leaves.map(leaf => amountIn(reply, leaf.runs, window.from, window.hour));
  const unresolved = reply.leaves.some((leaf, index) => leaf.domain == null && amounts[index].exact !== 0n);
  const unresolvedAtEnd = reply.leaves.some((leaf, index) => leaf.domain == null && finals[index].exact !== 0n);
  const items: SunburstItem[] = reply.leaves.map((leaf, index) => {
    const amount = amounts[index];
    const final = finals[index];
    const observed = amount.exact == null || amount.exact !== 0n;
    const attribution = unresolved && leaf.domain != null;
    return {
      path: [
        { id: leaf.domain ?? UNRESOLVED_ARM, label: domainName(leaf.domain ?? UNRESOLVED_ARM) },
        { id: leaf.typeKey, label: leaf.name || unresolvedTypeName(leaf.typeKey) },
        { id: leaf.variant, label: variantNameAt(leaf.names, window.hour, leaf.variant) },
      ],
      value: amount.exact == null ? null : Number(amount.exact), exact: amount.exact,
      unknownValue: amount.exact == null, unknownLevels: [attribution, attribution, false],
      note: amount.reason ? unknownLabel(amount.reason) : attribution ? ATTRIBUTION_NOTE : undefined,
      final: final.geometry, finalUnknown: final.exact == null,
      finalUnknownLevels: [unresolvedAtEnd && leaf.domain != null, unresolvedAtEnd && leaf.domain != null, false],
      present: observed || attribution, counted: observed ? [leaf.variant] : [],
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
export const formatDeliveredIc = (value: number, exact: bigint | null, decimals = 5) =>
  formatAmount(exact ?? BigInt(Math.round(value)), decimals, decimals);

export function spendingFootnote(reply: ProductionSpendingReply, window: SpendingWindow, growth: 'share' | 'fill' = 'share'): string {
  const observation = reply.observations[0] ?? null;
  const since = observation == null ? '' : ` Counting began ${formatDateTime(observation)}.`;
  const read = readNote(readAt(reply.reads, window.hour), window.hour, 'production output is read once a day.');
  const total = amountIn(reply, reply.total, window.from, window.hour);
  const why = total.exact == null ? ` Total not known: ${unknownLabel(total.reason ?? 'outsideRead')}.` : '';
  const finalUnknown = amountIn(reply, reply.total, window.from, window.end).exact == null;
  return `Delivered units valued at the line’s cost when read; earlier deliveries are never repriced. Excludes progress in flight and deliveries before counting began.${since} Gaps and restarts leave whole-campaign totals not known.${why}${read ? ` ${read}` : ''}${finalUnknown && growth === 'fill' ? ' Fill uses Share while the whole total is not known.' : ''}`;
}

import type { DockyardAmount, DockyardMonth, DockyardSpendingReply, ProductionSpendingUnknown } from '../../generated/dashboard';
import type { SunburstItem, SunburstKey } from '../../charts/sunburst/layout';
import type { ColumnPeriod } from '../../charts/columns';
import { UNRESOLVED_ARM, UNRESOLVED_MARK, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { gameDate, monthNames } from '../../dashboard/calendar.ts';
import { amountIn, spendingFinals, UNKNOWN_LABELS, unknownLabel, validateProductionSpendingReply } from '../production-spending/spendingData.ts';
import type { SpendingWindow } from '../production-spending/spendingData';

export const LEVELS = ['purposeLevel', 'typeLevel', 'variantLevel'] as const;

/** What dockyard IC was spent on, in the order the chart and columns show them. */
export const PURPOSES = [
  { id: 'new', label: 'New ships' },
  { id: 'refit', label: 'Refits' },
  { id: 'repair', label: 'Repairs' },
] as const;
export type Purpose = typeof PURPOSES[number]['id'];

export const REPAIR_NOTE = 'Dockyard output used for repairs.';
const UNATTRIBUTED: SunburstKey = { id: 'unattributed', label: 'Unattributed', last: true };
const TYPE_NOTE = 'Some designs’ types are unknown.';
const PURPOSE_NOTE = 'Whether some of it was refits was not recorded.';
const SPLIT_NOTE = 'Its split by design is not known here.';
/** Why a month's whole is not known when designs content cannot resolve could be naval. */
const UNRESOLVED_REASON = 'unresolved';
const UNRESOLVED_KEY: SunburstKey = { id: UNRESOLVED_ARM, label: domainName(UNRESOLVED_ARM), last: true, keepZero: true };
const UNPROVEN_KEY: SunburstKey = { id: 'purpose-unrecorded', label: 'Purpose not recorded', last: true, keepZero: true };
const UNREAD_KEY: SunburstKey = { id: 'not-known', label: 'Not known', last: true };

const RAW = /^-?\d{1,39}$/;
const int = (value: unknown): value is number => Number.isSafeInteger(value);
const amount = (value: unknown): value is DockyardAmount => {
  const item = value as DockyardAmount;
  return typeof item === 'object' && item != null
    && (item.totalRaw === null ? Object.hasOwn(UNKNOWN_LABELS, item.reason as string)
      : typeof item.totalRaw === 'string' && RAW.test(item.totalRaw) && item.reason === null);
};

/** Production spending's shape, its leaves split by purpose, with the repairs
 * and months a dockyard reply adds. */
export function validateDockyardSpendingReply(data: unknown): DockyardSpendingReply {
  const reply = validateProductionSpendingReply(data) as DockyardSpendingReply;
  const ok = reply.leaves.every(leaf => typeof leaf.refit === 'boolean' && !leaf.rocket)
    && typeof reply.repairs === 'object' && reply.repairs != null && amount(reply.repairs.hour) && amount(reply.repairs.end)
    && (reply.repairs.since === null || int(reply.repairs.since))
    && int(reply.repairs.denominator) && reply.repairs.denominator > 0
    && Array.isArray(reply.repairs.parts) && reply.repairs.parts.every(part => typeof part === 'object' && part != null
      && (part.variant === null ? part.typeKey === '' : typeof part.variant === 'string' && part.variant.length > 0 && part.typeKey.length > 0)
      && (part.name === null || typeof part.name === 'string') && Array.isArray(part.names) && amount(part.hour) && amount(part.end))
    && new Set(reply.repairs.parts.map(part => part.variant)).size === reply.repairs.parts.length
    && Array.isArray(reply.months) && reply.months.every((month, index) => typeof month === 'object' && month != null
      && int(month.firstHour) && int(month.lastHour) && month.firstHour <= month.lastHour
      && (index === 0 || reply.months[index - 1].lastHour < month.firstHour)
      && amount(month.newShips) && amount(month.refits) && amount(month.repairs)
      && typeof month.purposeUnproven === 'boolean' && typeof month.unresolved === 'boolean');
  if (!ok) throw new Error('The dockyard spending reply is malformed.');
  return reply;
}

/** An exact amount, a fixed5 numerator over `denominator` when it has one. */
export interface Exact { exact: bigint | null; reason: string | null; denominator?: bigint }
export const exactOf = (value: DockyardAmount): Exact =>
  ({ exact: value.totalRaw == null ? null : BigInt(value.totalRaw), reason: value.reason });

/** One moment's repair amounts by part, keyed by design (null for the
 * unattributed part). Where the split is not known but the total is, the
 * total stays: the known designs keep theirs, the unattributed part takes the
 * rest and the split is marked incomplete. */
function repairSplit(total: Exact, parts: readonly Exact[], unattributed: number): { values: Exact[]; incomplete: boolean } {
  if (total.exact == null || parts.every(part => part.exact != null)) return { values: [...parts], incomplete: false };
  const known = parts.reduce((sum, part, index) => index === unattributed || part.exact == null ? sum : sum + part.exact, 0n);
  return {
    values: parts.map((part, index) => index === unattributed ? { exact: total.exact! - known, reason: null }
      : part.exact == null ? { exact: null, reason: null } : part),
    incomplete: true,
  };
}

/** The repairs wedge: each repaired design's share of the dockyards the
 * game's repair selection placed at each read, by equipment type and design,
 * and the dockyards it did not place as one unattributed part. A reply
 * without parts holds the whole amount unattributed. */
export function repairItems(reply: DockyardSpendingReply, hour: number, purpose: SunburstKey = PURPOSES[2]): SunburstItem[] {
  const whole = { variant: null, typeKey: '', name: null, domain: null, names: [], hour: reply.repairs.hour, end: reply.repairs.end };
  const parts = [...reply.repairs.parts.filter(part => part.variant != null),
    reply.repairs.parts.find(part => part.variant == null) ?? whole];
  const rest = parts.length - 1;
  // Parts and total share one denominator, so they add exactly.
  const denominator = BigInt(reply.repairs.denominator);
  const now = repairSplit(exactOf(reply.repairs.hour), parts.map(part => exactOf(part.hour)), rest);
  const end = repairSplit(exactOf(reply.repairs.end), parts.map(part => exactOf(part.end)), rest);
  const since = reply.repairs.since == null ? REPAIR_NOTE : `${REPAIR_NOTE.slice(0, -1)} since ${dayLabel(reply.repairs.since)}.`;
  return parts.map((part, index) => {
    const value = now.values[index], final = end.values[index];
    const path: SunburstKey[] = part.variant == null ? [purpose, UNATTRIBUTED] : [purpose,
      { id: part.typeKey, label: part.name ?? unresolvedTypeName(part.typeKey) },
      { id: part.variant, label: variantNameAt(part.names, hour, part.variant) }];
    // A design whose share is withdrawn is not measured; its amount is in the rest.
    const withdrawn = now.incomplete && value.exact == null;
    const observed = !withdrawn && (value.exact == null || value.exact !== 0n);
    const reason = exactOf(part.hour).reason ?? exactOf(reply.repairs.hour).reason;
    return {
      path, value: value.exact == null ? null : Number(value.exact) / Number(denominator), exact: value.exact, exactDenominator: denominator,
      unknownValue: value.exact == null && !withdrawn,
      // A refused split leaves the known total and the rest exact: only a
      // design's equipment type misses shares, never Repairs or the whole.
      localUnknownLevels: [false, now.incomplete && part.variant != null, false],
      localFinalUnknownLevels: [false, end.incomplete && part.variant != null, false],
      note: now.incomplete ? `${since} ${SPLIT_NOTE}` : reason ? `${since} ${unknownLabel(reason)}` : since,
      final: final.exact == null ? 0 : Number(final.exact) / Number(denominator), finalUnknown: final.exact == null && !end.incomplete,
      present: observed, counted: observed && part.variant != null ? [part.variant] : [],
    };
  });
}

/** New ships and refits by equipment type and design, then repairs. A design
 * whose type content cannot resolve could be naval: it is left out, and the
 * purpose it belongs to is marked incompletely attributed. Input from a line
 * whose purpose is not proved counts as new, and marks new and refit
 * attribution incomplete. */
export function dockyardItems(reply: DockyardSpendingReply, window: SpendingWindow, colours: readonly string[] = []): SunburstItem[] {
  // Each purpose keeps its colour in both views, whatever its size.
  const purposes: SunburstKey[] = PURPOSES.map((purpose, index) => colours[index] ? { ...purpose, colour: colours[index] } : purpose);
  const finals = spendingFinals(reply, window.from, window.end);
  const amounts = reply.leaves.map(leaf => amountIn(reply, leaf.runs, window.from, window.hour, reply.equipmentObservations, 'equipment'));
  const unresolved = (leaf: DockyardSpendingReply['leaves'][number]) => leaf.typeKey.startsWith(UNRESOLVED_MARK);
  const missing = (at: 'now' | 'end', refit: boolean) => reply.leaves.some((leaf, index) => unresolved(leaf) && leaf.refit === refit
    && (at === 'now' ? amounts[index].exact : finals[index].exact) !== 0n);
  // Whether input of lines whose purpose is not recorded joined a leaf over
  // the dates shown, read against the same baseline as its amounts.
  const unprovenIn = (leaf: DockyardSpendingReply['leaves'][number], until: number) => leaf.unproven.length > 0
    && amountIn(reply, leaf.unproven, window.from, until, reply.equipmentObservations, 'equipment').exact !== 0n;
  const purposeNow = reply.leaves.some(leaf => unprovenIn(leaf, window.hour));
  const purposeEnd = reply.leaves.some(leaf => unprovenIn(leaf, window.end));
  const items: SunburstItem[] = [];
  reply.leaves.forEach((leaf, index) => {
    if (unresolved(leaf)) return;
    const refit = leaf.refit === true;
    const value = amounts[index], final = finals[index];
    const typeNow = missing('now', refit), typeEnd = missing('end', refit);
    const ownPurpose = unprovenIn(leaf, window.hour);
    // Unrecorded-purpose input keeps a design present even where it nets to zero.
    const observed = value.exact == null || value.exact !== 0n || ownPurpose;
    items.push({
      path: [
        purposes[refit ? 1 : 0],
        { id: leaf.typeKey, label: leaf.name ?? unresolvedTypeName(leaf.typeKey) },
        { id: leaf.variant, label: variantNameAt(leaf.names, window.hour, leaf.variant) },
      ],
      value: value.exact == null ? null : Number(value.exact), exact: value.exact, unknownValue: value.exact == null,
      // A design that could be naval leaves the whole uncertain; unproven
      // purpose only splits New ships and Refits, whose sum stays known.
      unknownLevels: [typeNow, false, false],
      finalUnknownLevels: [typeEnd, false, false],
      localUnknownLevels: [purposeNow, false, false],
      localFinalUnknownLevels: [purposeEnd, false, false],
      note: value.reason ? unknownLabel(value.reason) : ownPurpose ? PURPOSE_NOTE : typeNow ? TYPE_NOTE : undefined,
      final: final.geometry, finalUnknown: final.exact == null,
      present: observed, counted: observed ? [`${leaf.variant}`] : [],
    });
  });
  // Equipment spending the source could not read is not zero: each purpose
  // holds an unknown part, whatever Rocket coverage holds.
  const equipment = (until: number) => amountIn(reply, reply.equipmentTotal, window.from, until, reply.equipmentObservations, 'equipment');
  const unreadNow = equipment(window.hour), unreadEnd = equipment(window.end);
  if (unreadNow.exact == null || unreadEnd.exact == null) for (const purpose of purposes.slice(0, 2)) items.push({
    path: [purpose, UNREAD_KEY], value: null, exact: null, unknownValue: unreadNow.exact == null,
    note: unknownLabel(unreadNow.reason ?? unreadEnd.reason ?? 'outsideRead'), final: 0, finalUnknown: unreadEnd.exact == null,
    present: unreadNow.exact == null,
  });
  // Input of lines whose purpose is not recorded keeps both purposes'
  // uncertainty present, whatever its net amount.
  for (const purpose of purposes.slice(0, 2)) {
    if (!purposeNow && !purposeEnd) break;
    items.push({
      path: [purpose, UNPROVEN_KEY], value: 0, exact: 0n,
      localUnknownLevels: [purposeNow, false, false], localFinalUnknownLevels: [purposeEnd, false, false],
      note: PURPOSE_NOTE, final: 0, present: purposeNow,
    });
  }
  // Spending on a design that could be naval is kept present under its
  // purpose, so the chart never says nothing was spent while it is unknown.
  for (const refit of [false, true]) {
    if (!missing('now', refit) && !missing('end', refit)) continue;
    items.push({
      path: [purposes[refit ? 1 : 0], UNRESOLVED_KEY], value: 0, exact: 0n,
      unknownLevels: [missing('now', refit), true, false], finalUnknownLevels: [missing('end', refit), true, false],
      note: TYPE_NOTE, final: 0, present: missing('now', refit),
    });
  }
  return [...items, ...repairItems(reply, window.hour, purposes[2])];
}

/** Whether the reply holds any known repair amount: without production
 * reads, known repairs still show, beside unknown new ships and refits. */
export const repairsRead = (reply: DockyardSpendingReply) => reply.repairs.hour.totalRaw != null
  || reply.months.some(month => month.repairs.totalRaw != null);

/** One column per month, its bands in [`PURPOSES`] order, as exact amounts
 * divided by the recorded scale (and repairs' denominator) for drawing. */
export function monthColumns(months: readonly DockyardMonth[], decimals: number, repairDenominator: number): ColumnPeriod[] {
  const scale = 10 ** decimals;
  const drawn = (value: DockyardAmount, denominator = 1) => value.totalRaw == null ? null : Number(value.totalRaw) / denominator / scale;
  return months.map(month => ({
    firstHour: month.firstHour, lastHour: month.lastHour,
    values: [drawn(month.newShips), drawn(month.refits), drawn(month.repairs, repairDenominator)],
    uncertain: [month.purposeUnproven || month.unresolved, month.purposeUnproven || month.unresolved, false],
  }));
}

/** A month's name and year, from the engine calendar. */
export function monthLabel(hour: number): string {
  const date = gameDate(hour);
  return date ? `${monthNames[date.month - 1]} ${date.year}` : 'Unavailable date';
}

/** A month's total over the repairs' denominator, known only when each of
 * its parts is and no design that could be naval is left out of them; an
 * unrecorded purpose only splits new ships and refits. */
export function monthTotal(month: DockyardMonth, repairDenominator: number): Exact {
  if (month.unresolved) return { exact: null, reason: UNRESOLVED_REASON };
  const parts = [month.newShips, month.refits, month.repairs].map(exactOf);
  const unknown = parts.find(part => part.exact == null);
  if (unknown) return { exact: null, reason: unknown.reason };
  const denominator = BigInt(repairDenominator);
  const [ships, refits, repairs] = parts.map(part => part.exact as bigint);
  return { exact: (ships + refits) * denominator + repairs, reason: null, denominator };
}

/** `numerator` / `denominator` rounded to the nearest integer, halves away
 * from zero. */
export function nearestInteger(numerator: bigint, denominator = 1n): bigint {
  if (denominator === 1n) return numerator;
  const half = denominator / 2n;
  return numerator < 0n ? -((-numerator + half) / denominator) : (numerator + half) / denominator;
}

/** An exact IC amount of `decimals` recorded places, over `denominator`,
 * written to `digits` places: rounded once, straight from the exact
 * rational, never through the recorded places first. */
export function formatExactIc(exact: bigint, denominator: bigint, decimals: number, digits: number): string {
  const shown = Math.max(0, Math.min(digits, decimals));
  return formatAmount(nearestInteger(exact * 10n ** BigInt(shown), denominator * 10n ** BigInt(decimals)), shown, shown);
}

/** Why a month's part is not known, or what makes its attribution incomplete. */
export function monthNote(month: DockyardMonth): string | undefined {
  if (month.purposeUnproven) return PURPOSE_NOTE;
  if (month.unresolved) return TYPE_NOTE;
  return undefined;
}

export const reasonLabel = (reason: string | null) => reason === UNRESOLVED_REASON ? TYPE_NOTE
  : unknownLabel((reason ?? 'outsideRead') as ProductionSpendingUnknown);

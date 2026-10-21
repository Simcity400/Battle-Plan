import type { DockyardAmount, DockyardMonth, DockyardSpendingReply, ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingUnknown } from '../../generated/dashboard';
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
export const TYPE_NOTE = 'Some designs’ types are unknown.';
const PURPOSE_NOTE = 'Whether some of it was refits was not recorded.';
/** The leaves a dockyard reply splits its production into. */
const DOCKYARD_SPLIT: PurposeSplit = { purposes: [{ key: PURPOSES[0], leaf: 'new' }, { key: PURPOSES[1], leaf: 'refit' }], note: PURPOSE_NOTE };
const SPLIT_NOTE = 'Its split by design is not known here.';
/** Why a month's whole is not known when designs content cannot resolve could be naval. */
export const UNRESOLVED_REASON = 'unresolved';
const UNRESOLVED_KEY: SunburstKey = { id: UNRESOLVED_ARM, label: domainName(UNRESOLVED_ARM), last: true, keepZero: true };
/** Input whose purpose was not recorded, and input of a purpose the
 * equipment never has: parts of the whole in both views. */
export const UNRECORDED: SunburstKey = { id: 'purpose-unrecorded', label: 'Purpose not recorded', last: true, keepZero: true };
export const OTHER: SunburstKey = { id: 'other-purpose', label: 'Other purposes', last: true };
/** The ring's parts after the purposes and the months' bands after theirs. */
export const REMAINDER_BANDS = [UNRECORDED, OTHER] as const;
const UNRECORDED_NOTE = 'Its purpose was not recorded.';
const OTHER_NOTE = 'Recorded with a purpose this equipment never has.';
const UNREAD_KEY: SunburstKey = { id: 'not-known', label: 'Not known', last: true };

const RAW = /^-?\d{1,39}$/;
const int = (value: unknown): value is number => Number.isSafeInteger(value);
export const validAmount = (value: unknown): value is DockyardAmount => {
  const item = value as DockyardAmount;
  return typeof item === 'object' && item != null
    && (item.totalRaw === null ? Object.hasOwn(UNKNOWN_LABELS, item.reason as string)
      : typeof item.totalRaw === 'string' && RAW.test(item.totalRaw) && item.reason === null);
};

/** Production spending's shape, its leaves split by purpose, with the repairs
 * and months a dockyard reply adds. */
export function validateDockyardSpendingReply(data: unknown): DockyardSpendingReply {
  const reply = validateProductionSpendingReply(data) as DockyardSpendingReply;
  const ok = splitLeaves(reply)
    && typeof reply.repairs === 'object' && reply.repairs != null && validAmount(reply.repairs.hour) && validAmount(reply.repairs.end)
    && (reply.repairs.since === null || int(reply.repairs.since))
    && int(reply.repairs.denominator) && reply.repairs.denominator > 0
    && Array.isArray(reply.repairs.parts) && reply.repairs.parts.every(part => typeof part === 'object' && part != null
      && (part.variant === null ? part.typeKey === '' : typeof part.variant === 'string' && part.variant.length > 0 && part.typeKey.length > 0)
      && (part.name === null || typeof part.name === 'string') && Array.isArray(part.names) && validAmount(part.hour) && validAmount(part.end))
    && new Set(reply.repairs.parts.map(part => part.variant)).size === reply.repairs.parts.length
    && Array.isArray(reply.months) && reply.months.every((month, index) => typeof month === 'object' && month != null
      && int(month.firstHour) && int(month.lastHour) && month.firstHour <= month.lastHour
      && (index === 0 || reply.months[index - 1].lastHour < month.firstHour)
      && validAmount(month.newShips) && validAmount(month.refits) && validAmount(month.repairs)
      && validAmount(month.unrecorded) && validAmount(month.other)
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

/** Applied IC split by purpose: each shown ring purpose holds the leaves of
 * one reply purpose; `note` says which split an unrecorded purpose leaves
 * open. */
export interface PurposeSplit {
  purposes: readonly { key: SunburstKey; leaf: NonNullable<ProductionSpendingLeaf['purpose']> }[];
  note: string;
  /** The [`REMAINDER_BANDS`] parts' keys, coloured as the months draw them. */
  remainders?: readonly [SunburstKey, SunburstKey];
}

/** Whether every leaf is equipment input split by purpose. */
export const splitLeaves = (reply: ProductionSpendingReply) => reply.leaves.every(leaf => !leaf.rocket && leaf.purpose != null);

/** The ring purpose holding a leaf: one the split shows, input whose purpose
 * was not recorded, or another purpose the equipment never has. */
function purposeOf(split: PurposeSplit, purpose: ProductionSpendingLeaf['purpose']): SunburstKey {
  const [unrecorded, other] = split.remainders ?? REMAINDER_BANDS;
  if (purpose === 'unknown') return unrecorded;
  return split.purposes.find(shown => shown.leaf === purpose)?.key ?? other;
}

/** Each purpose's applied IC by equipment type and design. Input whose
 * purpose was not recorded is its own part, never another purpose's amount;
 * where it joined the dates shown, every shown purpose is marked incompletely
 * attributed and the part is uncertain down its subtree, while the whole stays
 * known. Input of a purpose the equipment never has is an Other purposes part
 * of the whole. A design whose type content cannot resolve could belong to
 * the reply's equipment: it is left out, and its purpose is marked
 * incompletely attributed. */
export function purposeItems(reply: ProductionSpendingReply, window: SpendingWindow, split: PurposeSplit): SunburstItem[] {
  const finals = spendingFinals(reply, window.from, window.end);
  const amounts = reply.leaves.map(leaf => amountIn(reply, leaf.runs, window.from, window.hour, reply.equipmentObservations, 'equipment'));
  const unresolved = (leaf: ProductionSpendingLeaf) => leaf.typeKey.startsWith(UNRESOLVED_MARK);
  const missing = (at: 'now' | 'end', key: SunburstKey) => reply.leaves.some((leaf, index) => unresolved(leaf)
    && purposeOf(split, leaf.purpose) === key && (at === 'now' ? amounts[index].exact : finals[index].exact) !== 0n);
  // Whether input whose purpose is not recorded joined a leaf over the dates
  // shown, read against the same baseline as its amounts: its summed
  // magnitudes, which never cancel.
  const unprovenIn = (leaf: ProductionSpendingLeaf, until: number) => leaf.unproven.length > 0
    && amountIn(reply, leaf.unproven, window.from, until, reply.equipmentObservations, 'equipment').exact !== 0n;
  const purposeNow = reply.leaves.some(leaf => unprovenIn(leaf, window.hour));
  const purposeEnd = reply.leaves.some(leaf => unprovenIn(leaf, window.end));
  const shown = new Set(split.purposes.map(purpose => purpose.key));
  // Shown purposes may lack unrecorded input; the unrecorded part is
  // uncertain at every ring.
  const local = (key: SunburstKey, uncertain: boolean) => key.id === UNRECORDED.id ? [uncertain, uncertain, uncertain]
    : [uncertain && shown.has(key), false, false];
  const items: SunburstItem[] = [];
  reply.leaves.forEach((leaf, index) => {
    if (unresolved(leaf)) return;
    const key = purposeOf(split, leaf.purpose);
    const value = amounts[index], final = finals[index];
    const typeNow = missing('now', key), typeEnd = missing('end', key);
    const ownPurpose = unprovenIn(leaf, window.hour);
    // Unrecorded-purpose input keeps a design present even where it nets to zero.
    const observed = value.exact == null || value.exact !== 0n || ownPurpose;
    items.push({
      path: [
        key,
        { id: leaf.typeKey, label: leaf.name ?? unresolvedTypeName(leaf.typeKey) },
        { id: leaf.variant, label: variantNameAt(leaf.names, window.hour, leaf.variant) },
      ],
      value: value.exact == null ? null : Number(value.exact), exact: value.exact, unknownValue: value.exact == null,
      // An unresolved design leaves the whole uncertain; an unrecorded
      // purpose only splits the purposes, whose sum stays known.
      unknownLevels: [typeNow, false, false],
      finalUnknownLevels: [typeEnd, false, false],
      localUnknownLevels: local(key, purposeNow),
      localFinalUnknownLevels: local(key, purposeEnd),
      note: value.reason ? unknownLabel(value.reason) : key.id === UNRECORDED.id ? UNRECORDED_NOTE : key.id === OTHER.id ? OTHER_NOTE
        : purposeNow ? split.note : typeNow ? TYPE_NOTE : undefined,
      final: final.geometry, finalUnknown: final.exact == null,
      present: observed, counted: observed ? [`${leaf.variant}`] : [],
    });
  });
  // Equipment spending the source could not read is not zero: each purpose
  // holds an unknown part, whatever Rocket coverage holds.
  const equipment = (until: number) => amountIn(reply, reply.equipmentTotal, window.from, until, reply.equipmentObservations, 'equipment');
  const unreadNow = equipment(window.hour), unreadEnd = equipment(window.end);
  if (unreadNow.exact == null || unreadEnd.exact == null) for (const { key } of split.purposes) items.push({
    path: [key, UNREAD_KEY], value: null, exact: null, unknownValue: unreadNow.exact == null,
    note: unknownLabel(unreadNow.reason ?? unreadEnd.reason ?? 'outsideRead'), final: 0, finalUnknown: unreadEnd.exact == null,
    present: unreadNow.exact == null,
  });
  // Spending on an unresolved design is kept present under its purpose, so
  // the chart never says nothing was spent while it is unknown: under a shown
  // purpose, the unrecorded part or Other purposes, whose whole it leaves
  // uncertain.
  for (const key of [...shown, purposeOf(split, 'unknown'), purposeOf(split, null)]) {
    if (!missing('now', key) && !missing('end', key)) continue;
    items.push({
      path: [key, UNRESOLVED_KEY], value: 0, exact: 0n,
      unknownLevels: [missing('now', key), true, false], finalUnknownLevels: [missing('end', key), true, false],
      note: TYPE_NOTE, final: 0, present: missing('now', key),
    });
  }
  return items;
}

/** Each purpose keeps its colour in both views, whatever its size. */
export const coloured = (key: SunburstKey, colour: string | undefined): SunburstKey => colour ? { ...key, colour } : key;

/** `split` with each purpose and remainder in its colour from `colours`, in
 * [`PURPOSES`] then [`REMAINDER_BANDS`] order after `skip` other bands. */
export function colouredSplit(split: PurposeSplit, colours: readonly string[], skip = 0): PurposeSplit {
  const after = split.purposes.length + skip;
  return { ...split, purposes: split.purposes.map((purpose, index) => ({ ...purpose, key: coloured(purpose.key, colours[index]) })),
    remainders: [coloured(UNRECORDED, colours[after]), coloured(OTHER, colours[after + 1])] };
}

/** New ships and refits by equipment type and design, then repairs. */
export function dockyardItems(reply: DockyardSpendingReply, window: SpendingWindow, colours: readonly string[] = []): SunburstItem[] {
  return [...purposeItems(reply, window, colouredSplit(DOCKYARD_SPLIT, colours, 1)),
    ...repairItems(reply, window.hour, coloured(PURPOSES[2], colours[2]))];
}

/** The months' bands: dockyard IC's purposes, then the remainders. */
export const BANDS = [...PURPOSES, ...REMAINDER_BANDS];

/** Whether the reply holds any known repair amount: without production
 * reads, known repairs still show, beside unknown new ships and refits. */
export const repairsRead = (reply: DockyardSpendingReply) => reply.repairs.hour.totalRaw != null
  || reply.months.some(month => month.repairs.totalRaw != null);

/** One column per month, its bands in [`BANDS`] order, as exact amounts
 * divided by the recorded scale (and repairs' denominator) for drawing. */
export function monthColumns(months: readonly DockyardMonth[], decimals: number, repairDenominator: number): ColumnPeriod[] {
  const drawn = (value: DockyardAmount, denominator = 1) => drawnAmount(value, decimals, denominator);
  return months.map(month => {
    const purposes = month.purposeUnproven || month.unresolved;
    return {
      firstHour: month.firstHour, lastHour: month.lastHour,
      values: [drawn(month.newShips), drawn(month.refits), drawn(month.repairs, repairDenominator), drawn(month.unrecorded), drawn(month.other)],
      uncertain: [purposes, purposes, false, month.purposeUnproven, false],
    };
  });
}

/** An exact amount divided by the recorded scale and `denominator`, for drawing. */
export const drawnAmount = (value: DockyardAmount, decimals: number, denominator = 1) =>
  value.totalRaw == null ? null : Number(value.totalRaw) / denominator / 10 ** decimals;

/** The exact sum of `parts` over `denominator`, each part's own over it when
 * it has one; unknown when any part is, with its reason. */
export function exactSum(parts: readonly Exact[], denominator = 1n): Exact {
  let exact = 0n;
  for (const part of parts) {
    if (part.exact == null) return { exact: null, reason: part.reason };
    exact += part.denominator == null ? part.exact * denominator : part.exact;
  }
  return denominator === 1n ? { exact, reason: null } : { exact, reason: null, denominator };
}

/** A month's name and year, from the engine calendar. */
export function monthLabel(hour: number): string {
  const date = gameDate(hour);
  return date ? `${monthNames[date.month - 1]} ${date.year}` : 'Unavailable date';
}

/** A month's total over the repairs' denominator, known only when each of
 * its parts is and no design that could be naval is left out of them; an
 * unrecorded purpose only splits it. */
export function monthTotal(month: DockyardMonth, repairDenominator: number): Exact {
  if (month.unresolved) return { exact: null, reason: UNRESOLVED_REASON };
  const denominator = BigInt(repairDenominator);
  return exactSum([exactOf(month.newShips), exactOf(month.refits), { ...exactOf(month.repairs), denominator },
    exactOf(month.unrecorded), exactOf(month.other)], denominator);
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

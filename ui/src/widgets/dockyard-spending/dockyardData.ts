import type { SunburstItem, SunburstKey } from '../../charts/sunburst/layout';
import type { ColumnPeriod } from '../../charts/columns';
import { UNRESOLVED_ARM, UNRESOLVED_MARK, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { firstHourOfMonth, gameDate, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import { amountIn, spendingFinals, unknownLabel, validateProductionSpendingReply } from '../production-spending/spendingData.ts';
import type { SpendingData } from '../production-spending/spendingData';
import type { SpendingAmount, SpendingDesign, SpendingWindow } from '../production-spending/spendingData';
import type { Domain, ProductionPurpose, ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun, ProductionSpendingUnknown } from '../../generated/dashboard';

export const LEVELS = ['purposeLevel', 'typeLevel', 'variantLevel'] as const;

/** What dockyard IC was spent on, in the order the chart and columns show them. */
export const PURPOSES = [
  { id: 'new', label: 'New ships' },
  { id: 'refit', label: 'Refits' },
  { id: 'repair', label: 'Repairs' },
] as const;

export const REPAIR_NOTE = 'Dockyard output used for repairs.';
const UNATTRIBUTED: SunburstKey = { id: 'unattributed', label: 'Unattributed', last: true };
export const TYPE_NOTE = 'Some designs’ types are unknown.';
const PURPOSE_NOTE = 'Whether some of it was refits was not recorded.';
/** The leaves a dockyard reply splits its production into. */
const DOCKYARD_SPLIT: PurposeSplit = { purposes: [{ key: PURPOSES[0], leaf: 'new' }, { key: PURPOSES[1], leaf: 'refit' }], note: PURPOSE_NOTE };
const SPLIT_NOTE = 'Its split by design is not known here.';
/** Why a month's whole is not known when designs content cannot resolve could be the domain's. */
export const UNRESOLVED_REASON = 'unresolved';
const UNRESOLVED_KEY: SunburstKey = { id: UNRESOLVED_ARM, label: domainName(UNRESOLVED_ARM), last: true, keepZero: true };
/** Input whose purpose was not recorded, and input of a purpose the equipment never has: parts of the whole in
 * both views. */
export const UNRECORDED: SunburstKey = { id: 'purpose-unrecorded', label: 'Purpose not recorded', last: true, keepZero: true };
export const OTHER: SunburstKey = { id: 'other-purpose', label: 'Other purposes', last: true };
/** The ring's parts after the purposes and the months' bands after theirs. */
export const REMAINDER_BANDS = [UNRECORDED, OTHER] as const;
const UNRECORDED_NOTE = 'Its purpose was not recorded.';
const OTHER_NOTE = 'Recorded with a purpose this equipment never has.';
const UNREAD_KEY: SunburstKey = { id: 'not-known', label: 'Not known', last: true };

/** Production spending's reply with repair rates, its leaves split by purpose. */
export function validateDockyardSpendingReply(data: unknown): ProductionSpendingReply {
  const reply = validateProductionSpendingReply(data);
  if (!reply.repairs) throw new Error('The dockyard spending reply is malformed.');
  return reply;
}

/** An exact amount, a fixed5 numerator over `denominator` when it has one. */
export interface Exact { exact: bigint | null; reason: string | null; denominator?: bigint }
export const exactOf = (amount: SpendingAmount): Exact => ({ exact: amount.exact, reason: amount.reason });

/** Whether a leaf's type content does not resolve: it could be of any domain. */
export const unresolvedLeaf = (leaf: ProductionSpendingLeaf) => leaf.typeKey.startsWith(UNRESOLVED_MARK);

/** The leaves that could be `domain`'s: its own and those whose type content does not resolve, never Rocket's. */
export const domainLeaves = (reply: SpendingData, domain: Domain) =>
  reply.leaves.filter(leaf => !leaf.rocket && (leaf.domain === domain || unresolvedLeaf(leaf)));

/** A rate's runs indexed for exact sums over hours: each run's sum through its end, and how many runs not known
 * (or hours between runs) lie before it. */
interface RateIndex { runs: readonly ProductionSpendingRun[]; through: bigint[]; unknown: number[] }
const indexes = new WeakMap<readonly ProductionSpendingRun[], RateIndex>();

function rateIndex(runs: readonly ProductionSpendingRun[]): RateIndex {
  const held = indexes.get(runs);
  if (held) return held;
  const through: bigint[] = [], unknown: number[] = [];
  let sum = 0n, missing = 0;
  runs.forEach((run, index) => {
    if (index > 0 && runs[index - 1].lastHour + 1 !== run.firstHour) missing++;
    if (run.totalRaw == null) missing++;
    else sum += BigInt(run.totalRaw) * BigInt(run.lastHour - run.firstHour + 1);
    through.push(sum);
    unknown.push(missing);
  });
  const index = { runs, through, unknown };
  indexes.set(runs, index);
  return index;
}

const runIndex = (runs: readonly ProductionSpendingRun[], hour: number) => {
  let low = 0, high = runs.length - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (hour < runs[mid].firstHour) high = mid - 1;
    else if (hour > runs[mid].lastHour) low = mid + 1;
    else return mid;
  }
  return -1;
};

/** A rate's runs (fixed5 a day) summed over `first..=last`, over `denominator` (the hours of a day): unknown
 * where an hour holds no known rate, with its reason. */
export function rateSum(runs: readonly ProductionSpendingRun[], first: number, last: number, denominator: number): Exact {
  const over = BigInt(denominator);
  if (last < first) return { exact: 0n, reason: null, denominator: over };
  const a = runIndex(runs, first), b = runIndex(runs, last);
  if (a < 0 || b < 0) return { exact: null, reason: 'outsideRead' };
  const index = rateIndex(runs);
  if (index.unknown[b] - index.unknown[a] > 0 || runs[a].totalRaw == null) {
    const missing = runs.slice(a, b + 1).find(run => run.totalRaw == null);
    return { exact: null, reason: missing?.reason ?? 'outsideRead' };
  }
  const at = (run: number, hour: number) => index.through[run] - BigInt(runs[run].totalRaw!) * BigInt(runs[run].lastHour - hour);
  const before = at(a, first) - BigInt(runs[a].totalRaw!);
  return { exact: at(b, last) - before, reason: null, denominator: over };
}

/** The first hour the country's repair output is known: a whole campaign's repairs count from there. */
export const repairsSince = (reply: SpendingData) => reply.repairs?.total.find(run => run.totalRaw != null)?.firstHour ?? null;

/** One moment's repair amounts by part, keyed by design (null for the unattributed part). Where the split is not
 * known but the total is, the total stays: the known designs keep theirs, the unattributed part takes the rest
 * and the split is marked incomplete. */
function repairSplit(total: Exact, parts: readonly Exact[], unattributed: number): { values: Exact[]; incomplete: boolean } {
  if (total.exact == null || parts.every(part => part.exact != null)) return { values: [...parts], incomplete: false };
  const known = parts.reduce((sum, part, index) => index === unattributed || part.exact == null ? sum : sum + part.exact, 0n);
  return {
    values: parts.map((part, index) => index === unattributed ? { exact: total.exact! - known, reason: null, denominator: total.denominator }
      : part.exact == null ? { exact: null, reason: null } : part),
    incomplete: true,
  };
}

/** Repairs over a window: from its first hour, or, for a whole campaign, from the first hour repair output is
 * known, through `until`. */
export function repairAmount(reply: SpendingData, runs: readonly ProductionSpendingRun[], window: SpendingWindow, until: number): Exact {
  const since = window.from ?? repairsSince(reply);
  if (since == null || !reply.repairs) return { exact: null, reason: 'notRecorded' };
  return rateSum(runs, since, until, reply.repairs.denominator);
}

/** The repairs wedge: each repaired design's share of the dockyards the game's repair selection placed at each
 * hour, by equipment type and design, and the dockyards it did not place as one unattributed part. */
export function repairItems(reply: SpendingData, window: SpendingWindow, purpose: SunburstKey = PURPOSES[2]): SunburstItem[] {
  const repairs = reply.repairs;
  if (!repairs) return [];
  const whole = { variant: null, typeKey: '', name: null, domain: null, names: [], runs: repairs.total };
  const parts = [...repairs.parts.filter(part => part.variant != null), repairs.parts.find(part => part.variant == null) ?? whole];
  const rest = parts.length - 1;
  const denominator = BigInt(repairs.denominator);
  const now = repairSplit(repairAmount(reply, repairs.total, window, window.hour), parts.map(part => repairAmount(reply, part.runs, window, window.hour)), rest);
  const end = repairSplit(repairAmount(reply, repairs.total, window, window.end), parts.map(part => repairAmount(reply, part.runs, window, window.end)), rest);
  const since = repairsSince(reply);
  const noted = window.from == null && since != null ? `${REPAIR_NOTE.slice(0, -1)} since ${dayLabel(since)}.` : REPAIR_NOTE;
  const totalReason = repairAmount(reply, repairs.total, window, window.hour).reason;
  return parts.map((part, index) => {
    const value = now.values[index], final = end.values[index];
    const path: SunburstKey[] = part.variant == null ? [purpose, UNATTRIBUTED] : [purpose,
      { id: part.typeKey, label: part.name ?? unresolvedTypeName(part.typeKey) },
      { id: part.variant, label: variantNameAt(part.names, window.hour, part.variant) }];
    // A design whose share is withdrawn is not measured; its amount is in the rest.
    const withdrawn = now.incomplete && value.exact == null;
    const observed = !withdrawn && (value.exact == null || value.exact !== 0n);
    const reason = value.reason ?? totalReason;
    return {
      path, value: value.exact == null ? null : Number(value.exact) / Number(denominator), exact: value.exact, exactDenominator: denominator,
      unknownValue: value.exact == null && !withdrawn,
      // A refused split leaves the known total and the rest exact: only a design's equipment type misses shares.
      localUnknownLevels: [false, now.incomplete && part.variant != null, false],
      localFinalUnknownLevels: [false, end.incomplete && part.variant != null, false],
      note: now.incomplete ? `${noted} ${SPLIT_NOTE}` : reason ? `${noted} ${unknownLabel(reason)}` : noted,
      final: final.exact == null ? 0 : Number(final.exact) / Number(denominator), finalUnknown: final.exact == null && !end.incomplete,
      present: observed, counted: observed && part.variant != null ? [part.variant] : [],
    };
  });
}

/** Applied IC split by purpose: each shown ring purpose holds the leaves of one reply purpose; `note` says which
 * split an unrecorded purpose leaves open. */
export interface PurposeSplit {
  purposes: readonly { key: SunburstKey; leaf: ProductionPurpose }[];
  note: string;
  /** The [`REMAINDER_BANDS`] parts' keys, coloured as the months draw them. */
  remainders?: readonly [SunburstKey, SunburstKey];
}

/** The ring purpose holding a leaf: one the split shows, input whose purpose was not recorded, or another
 * purpose the equipment never has. */
function purposeOf(split: PurposeSplit, purpose: ProductionPurpose | null): SunburstKey {
  const [unrecorded, other] = split.remainders ?? REMAINDER_BANDS;
  if (purpose === 'unknown') return unrecorded;
  return split.purposes.find(shown => shown.leaf === purpose)?.key ?? other;
}

const single = (leaf: ProductionSpendingLeaf): SpendingDesign =>
  ({ variant: leaf.variant, typeKey: leaf.typeKey, name: leaf.name, domain: leaf.domain, rocket: leaf.rocket, names: leaf.names, parts: [leaf] });

/** Each purpose's applied IC by equipment type and design, from `leaves` (the domain's and those content cannot
 * type). Input whose purpose was not recorded is its own part, never another purpose's amount; where it joined
 * the dates shown, every shown purpose is marked incompletely attributed and the part is uncertain down its
 * subtree, while the whole stays known. Input of a purpose the equipment never has is an Other purposes part of
 * the whole. A design whose type content cannot resolve could belong to the reply's equipment: it is left out,
 * and its purpose is marked incompletely attributed. */
export function purposeItems(reply: SpendingData, leaves: readonly ProductionSpendingLeaf[], window: SpendingWindow, split: PurposeSplit): SunburstItem[] {
  const observations = reply.equipmentObservations;
  const finals = spendingFinals(reply, leaves.map(single), window.from, window.end);
  const amounts = leaves.map(leaf => amountIn(leaf.runs, window.from, window.hour, observations));
  const missing = (at: 'now' | 'end', key: SunburstKey) => leaves.some((leaf, index) => unresolvedLeaf(leaf)
    && purposeOf(split, leaf.purpose) === key && (at === 'now' ? amounts[index].exact : finals[index].exact) !== 0n);
  // Whether input whose purpose is not recorded joined a leaf over the dates shown, read against the same
  // baseline as its amounts: its summed magnitudes, which never cancel.
  const unprovenIn = (leaf: ProductionSpendingLeaf, until: number) => leaf.unproven.length > 0
    && amountIn(leaf.unproven, window.from, until, observations).exact !== 0n;
  const purposeNow = leaves.some(leaf => unprovenIn(leaf, window.hour));
  const purposeEnd = leaves.some(leaf => unprovenIn(leaf, window.end));
  const shown = new Set(split.purposes.map(purpose => purpose.key));
  const local = (key: SunburstKey, uncertain: boolean) => key.id === UNRECORDED.id ? [uncertain, uncertain, uncertain]
    : [uncertain && shown.has(key), false, false];
  const items: SunburstItem[] = [];
  leaves.forEach((leaf, index) => {
    if (unresolvedLeaf(leaf)) return;
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
  // Equipment spending the source could not read is not zero: each purpose holds an unknown part.
  const equipment = (until: number) => amountIn(reply.equipmentTotal, window.from, until, observations);
  const unreadNow = equipment(window.hour), unreadEnd = equipment(window.end);
  if (unreadNow.exact == null || unreadEnd.exact == null) for (const { key } of split.purposes) items.push({
    path: [key, UNREAD_KEY], value: null, exact: null, unknownValue: unreadNow.exact == null,
    note: unknownLabel(unreadNow.reason ?? unreadEnd.reason ?? 'outsideRead'), final: 0, finalUnknown: unreadEnd.exact == null,
    present: unreadNow.exact == null,
  });
  // Spending on an unresolved design is kept present under its purpose, so the chart never says nothing was
  // spent while it is unknown.
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

/** `split` with each purpose and remainder in its colour from `colours`, in [`PURPOSES`] then [`REMAINDER_BANDS`]
 * order after `skip` other bands. */
export function colouredSplit(split: PurposeSplit, colours: readonly string[], skip = 0): PurposeSplit {
  const after = split.purposes.length + skip;
  return { ...split, purposes: split.purposes.map((purpose, index) => ({ ...purpose, key: coloured(purpose.key, colours[index]) })),
    remainders: [coloured(UNRECORDED, colours[after]), coloured(OTHER, colours[after + 1])] };
}

/** New ships and refits by equipment type and design, then repairs. */
export function dockyardItems(reply: SpendingData, window: SpendingWindow, colours: readonly string[] = []): SunburstItem[] {
  return [...purposeItems(reply, domainLeaves(reply, 'naval'), window, colouredSplit(DOCKYARD_SPLIT, colours, 1)),
    ...repairItems(reply, window, coloured(PURPOSES[2], colours[2]))];
}

/** The months' bands: dockyard IC's purposes, then the remainders. */
export const BANDS = [...PURPOSES, ...REMAINDER_BANDS];

/** Whether the reply holds any known repair rate: without production reads, known repairs still show. */
export const repairsRead = (reply: SpendingData) => repairsSince(reply) != null;

/** The calendar months overlapping `first..=last`, each clipped to it. */
export function monthsOf(first: number, last: number): { firstHour: number; lastHour: number }[] {
  const out: { firstHour: number; lastHour: number }[] = [];
  let at = first;
  while (at <= last) {
    const date = gameDate(at);
    if (!date) break;
    const end = Math.min(last, firstHourOfMonth(monthIndex(date) + 1) - 1);
    out.push({ firstHour: at, lastHour: end });
    at = end + 1;
  }
  return out;
}

/** One month's input of `leaves` per purpose, the change of each through its last hour; unknown where the lines'
 * whole or a resolved design's is, with its reason. Designs content cannot type only mark the month. A month the
 * branch starts in counts from its first read. */
export interface SpentMonth {
  amounts: Readonly<Record<ProductionPurpose, bigint>> | null;
  reason: string | null;
  purposeUnproven: boolean;
  unresolved: boolean;
}

export function spentMonth(reply: SpendingData, leaves: readonly ProductionSpendingLeaf[], first: number, last: number, branchFirst: number): SpentMonth {
  const observations = reply.equipmentObservations;
  const from = first <= branchFirst ? null : first;
  const whole = amountIn(reply.equipmentTotal, from, last, observations);
  const amounts: Record<ProductionPurpose, bigint> = { new: 0n, refit: 0n, converted: 0n, unknown: 0n };
  let reason: string | null = whole.exact == null ? whole.reason ?? 'outsideRead' : null;
  let purposeUnproven = false, unresolved = false;
  for (const leaf of leaves) {
    if (leaf.unproven.length && amountIn(leaf.unproven, from, last, observations).exact !== 0n) purposeUnproven = true;
    const change = amountIn(leaf.runs, from, last, observations);
    if (unresolvedLeaf(leaf)) {
      unresolved ||= change.exact !== 0n;
      continue;
    }
    if (change.exact == null) reason ??= change.reason ?? 'outsideRead';
    else if (leaf.purpose) amounts[leaf.purpose] += change.exact;
  }
  return { amounts: reason == null ? amounts : null, reason, purposeUnproven, unresolved };
}

/** One month's band amounts and what its tooltip adds. */
export interface PurposeMonth { firstHour: number; lastHour: number; bands: readonly Exact[]; total: Exact; note?: string; uncertain: readonly boolean[] }

const part = (month: SpentMonth, purposes: readonly ProductionPurpose[]): Exact => month.amounts
  ? { exact: purposes.reduce((sum, purpose) => sum + month.amounts![purpose], 0n), reason: null }
  : { exact: null, reason: month.reason };

/** A dockyard month: new ships, refits, repairs, the unrecorded purpose and the others. */
export function dockyardMonth(reply: SpendingData, first: number, last: number, branchFirst: number): PurposeMonth {
  const spent = spentMonth(reply, domainLeaves(reply, 'naval'), first, last, branchFirst);
  const repairs = reply.repairs ? rateSum(reply.repairs.total, first, last, reply.repairs.denominator) : { exact: null, reason: 'notRecorded' };
  const bands = [part(spent, ['new']), part(spent, ['refit']), repairs, part(spent, ['unknown']), part(spent, ['converted'])];
  const purposes = spent.purposeUnproven || spent.unresolved;
  return { firstHour: first, lastHour: last, bands, total: monthTotal(spent, bands, BigInt(reply.repairs?.denominator ?? 1)),
    note: monthNote(spent), uncertain: [purposes, purposes, false, spent.purposeUnproven, false] };
}

/** One column per month, its bands as exact amounts divided by the recorded scale (and a band's denominator)
 * for drawing. */
export function monthColumns(months: readonly PurposeMonth[], decimals: number): ColumnPeriod[] {
  return months.map(month => ({ firstHour: month.firstHour, lastHour: month.lastHour,
    values: month.bands.map(band => band.exact == null ? null : Number(band.exact) / Number(band.denominator ?? 1n) / 10 ** decimals),
    uncertain: month.uncertain }));
}

/** The exact sum of `parts` over `denominator`, each part's own over it when it has one; unknown when any part
 * is, with its reason. */
export function exactSum(parts: readonly Exact[], denominator = 1n): Exact {
  let exact = 0n;
  for (const part of parts) {
    if (part.exact == null) return { exact: null, reason: part.reason };
    exact += part.denominator == null ? part.exact * denominator : part.exact;
  }
  return denominator === 1n ? { exact, reason: null } : { exact, reason: null, denominator };
}

/** A month's total, known only when each of its parts is and no design that could be the domain's is left out
 * of them; an unrecorded purpose only splits it. */
export function monthTotal(month: SpentMonth, bands: readonly Exact[], denominator = 1n): Exact {
  if (month.unresolved) return { exact: null, reason: UNRESOLVED_REASON };
  return exactSum(bands, denominator);
}

/** A month's name and year, from the engine calendar. */
export function monthLabel(hour: number): string {
  const date = gameDate(hour);
  return date ? `${monthNames[date.month - 1]} ${date.year}` : 'Unavailable date';
}

/** `numerator` / `denominator` rounded to the nearest integer, halves away from zero. */
export function nearestInteger(numerator: bigint, denominator = 1n): bigint {
  if (denominator === 1n) return numerator;
  const half = denominator / 2n;
  return numerator < 0n ? -((-numerator + half) / denominator) : (numerator + half) / denominator;
}

/** An exact IC amount of `decimals` recorded places, over `denominator`, written to `digits` places: rounded
 * once, straight from the exact rational, never through the recorded places first. */
export function formatExactIc(exact: bigint, denominator: bigint, decimals: number, digits: number): string {
  const shown = Math.max(0, Math.min(digits, decimals));
  return formatAmount(nearestInteger(exact * 10n ** BigInt(shown), denominator * 10n ** BigInt(decimals)), shown, shown);
}

/** What makes a month's attribution incomplete. */
export function monthNote(month: SpentMonth, purposeNote = PURPOSE_NOTE): string | undefined {
  if (month.purposeUnproven) return purposeNote;
  if (month.unresolved) return TYPE_NOTE;
  return undefined;
}

export const reasonLabel = (reason: string | null) => reason === UNRESOLVED_REASON ? TYPE_NOTE
  : unknownLabel((reason ?? 'outsideRead') as ProductionSpendingUnknown);

import type { SunburstItem, SunburstKey } from '../../charts/sunburst/layout';
import { runAt } from '../../charts/series.ts';
import { UNRESOLVED_ARM, UNRESOLVED_MARK, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { amountIn, spendingFinals, unknownLabel, validateProductionSpendingReply } from '../production-spending/spendingData.ts';
import type { SpendingData, SpendingDesign, SpendingWindow } from '../production-spending/spendingData';
import type { Domain, ProductionPurpose, ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun, ProductionSpendingUnknown } from '../../generated/dashboard';

export const LEVELS = ['purposeLevel', 'typeLevel', 'variantLevel'] as const;

const PURPOSES = [
  { id: 'new', label: 'New ships' },
  { id: 'refit', label: 'Refits' },
  { id: 'repair', label: 'Repairs' },
] as const;

export const REPAIR_NOTE = 'Dockyard output used for repairs.';
const UNATTRIBUTED: SunburstKey = { id: 'unattributed', label: 'Unattributed', last: true };
const TYPE_NOTE = 'Some designs’ types are unknown.';
const PURPOSE_NOTE = 'Whether some of it was refits was not recorded.';
const DOCKYARD_SPLIT: PurposeSplit = { purposes: [{ key: PURPOSES[0], leaf: 'new' }, { key: PURPOSES[1], leaf: 'refit' }], note: PURPOSE_NOTE };
const SPLIT_NOTE = 'Its split by design is not known here.';
const UNRESOLVED_REASON = 'unresolved';
const UNRESOLVED_KEY: SunburstKey = { id: UNRESOLVED_ARM, label: domainName(UNRESOLVED_ARM), last: true, keepZero: true };
const UNRECORDED: SunburstKey = { id: 'purpose-unrecorded', label: 'Purpose not recorded', last: true, keepZero: true };
const OTHER: SunburstKey = { id: 'other-purpose', label: 'Other purposes', last: true };
export const REMAINDER_BANDS = [UNRECORDED, OTHER] as const;
const UNRECORDED_NOTE = 'Its purpose was not recorded.';
const OTHER_NOTE = 'Recorded with a purpose this equipment never has.';
const UNREAD_KEY: SunburstKey = { id: 'not-known', label: 'Not known', last: true };

export function validateDockyardSpendingReply(data: unknown): ProductionSpendingReply {
  const reply = validateProductionSpendingReply(data);
  if (!reply.repairs) throw new Error('The dockyard spending reply is malformed.');
  return reply;
}

export interface Exact { exact: bigint | null; reason: string | null; denominator?: bigint }

const unresolvedLeaf = (leaf: ProductionSpendingLeaf) => leaf.typeKey.startsWith(UNRESOLVED_MARK);

export const domainLeaves = (reply: SpendingData, domain: Domain) =>
  reply.leaves.filter(leaf => !leaf.rocket && (leaf.domain === domain || unresolvedLeaf(leaf)));

interface RateIndex { runs: readonly ProductionSpendingRun[]; through: bigint[]; unknown: number[]; nextMissing: number[] }
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
  const nextMissing = new Array<number>(runs.length);
  let next = -1;
  for (let i = runs.length - 1; i >= 0; i--) {
    if (runs[i].totalRaw == null) next = i;
    nextMissing[i] = next;
  }
  const index = { runs, through, unknown, nextMissing };
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

export function rateSum(runs: readonly ProductionSpendingRun[], first: number, last: number, denominator: number): Exact {
  const over = BigInt(denominator);
  if (last < first) return { exact: 0n, reason: null, denominator: over };
  const a = runIndex(runs, first), b = runIndex(runs, last);
  if (a < 0 || b < 0) return { exact: null, reason: 'outsideRead' };
  const index = rateIndex(runs);
  if (index.unknown[b] - index.unknown[a] > 0 || runs[a].totalRaw == null) {
    const miss = index.nextMissing[a];
    const reason = miss >= 0 && miss <= b ? runs[miss].reason : null;
    return { exact: null, reason: reason ?? 'outsideRead' };
  }
  const at = (run: number, hour: number) => index.through[run] - BigInt(runs[run].totalRaw!) * BigInt(runs[run].lastHour - hour);
  const before = at(a, first) - BigInt(runs[a].totalRaw!);
  return { exact: at(b, last) - before, reason: null, denominator: over };
}

export const repairsSince = (reply: SpendingData, atHour = Infinity) =>
  reply.repairs?.total.find(run => run.totalRaw != null && run.firstHour <= Math.floor(atHour))?.firstHour ?? null;

function repairSplit(total: Exact, parts: readonly Exact[], unattributed: number): { values: Exact[]; incomplete: boolean } {
  if (total.exact == null || parts.every(part => part.exact != null)) return { values: [...parts], incomplete: false };
  const known = parts.reduce((sum, part, index) => index === unattributed || part.exact == null ? sum : sum + part.exact, 0n);
  return {
    values: parts.map((part, index) => index === unattributed ? { exact: total.exact! - known, reason: null, denominator: total.denominator }
      : part.exact == null ? { exact: null, reason: null } : part),
    incomplete: true,
  };
}

export function repairAmount(reply: SpendingData, runs: readonly ProductionSpendingRun[], window: SpendingWindow, until: number): Exact {
  const at = Math.floor(until);
  const since = window.from ?? repairsSince(reply, at);
  if (since == null || !reply.repairs) return { exact: null, reason: runAt(runs, at)?.reason ?? 'notRecorded' };
  if (window.from == null && at < since) return { exact: null, reason: runAt(runs, at)?.reason ?? 'notRecorded' };
  return rateSum(runs, since, at, reply.repairs.denominator);
}

export function repairItems(reply: SpendingData, window: SpendingWindow, purpose: SunburstKey = PURPOSES[2]): SunburstItem[] {
  const repairs = reply.repairs;
  if (!repairs) return [];
  const whole = { variant: null, typeKey: '', name: null, domain: null, names: [], runs: repairs.total };
  const parts = [...repairs.parts.filter(part => part.variant != null), repairs.parts.find(part => part.variant == null) ?? whole];
  const rest = parts.length - 1;
  const denominator = BigInt(repairs.denominator);
  const futureEnd = window.end > window.hour;
  const totalNow = repairAmount(reply, repairs.total, window, window.hour);
  const now = repairSplit(totalNow, parts.map(part => repairAmount(reply, part.runs, window, window.hour)), rest);
  const end = futureEnd
    ? { values: parts.map(() => ({ exact: null, reason: null } as Exact)), incomplete: false }
    : repairSplit(repairAmount(reply, repairs.total, window, window.end), parts.map(part => repairAmount(reply, part.runs, window, window.end)), rest);
  const since = repairsSince(reply, window.hour);
  const noted = window.from == null && since != null ? `${REPAIR_NOTE.slice(0, -1)} since ${dayLabel(since)}.` : REPAIR_NOTE;
  const totalReason = totalNow.reason;
  return parts.map((part, index) => {
    const value = now.values[index], final = end.values[index];
    const path: SunburstKey[] = part.variant == null ? [purpose, UNATTRIBUTED] : [purpose,
      { id: part.typeKey, label: part.name ?? unresolvedTypeName(part.typeKey) },
      { id: part.variant, label: variantNameAt(part.names, window.hour, part.variant) }];
    const withdrawn = now.incomplete && value.exact == null;
    const observed = !withdrawn && (value.exact == null || value.exact !== 0n);
    const reason = value.reason ?? totalReason;
    const displayValue = value.exact == null ? null : Number(value.exact) / Number(denominator);
    return {
      path, value: displayValue, exact: value.exact, exactDenominator: denominator,
      unknownValue: value.exact == null && !withdrawn,
      localUnknownLevels: [false, now.incomplete && part.variant != null, false],
      localFinalUnknownLevels: [false, !futureEnd && end.incomplete && part.variant != null, false],
      note: now.incomplete ? `${noted} ${SPLIT_NOTE}` : reason ? `${noted} ${unknownLabel(reason)}` : noted,
      final: final.exact == null ? 0 : Number(final.exact) / Number(denominator),
      ...(futureEnd ? { weight: displayValue ?? 0 } : {}),
      finalUnknown: futureEnd || (final.exact == null && !end.incomplete),
      present: observed, counted: observed && part.variant != null ? [part.variant] : [],
    };
  });
}

export interface PurposeSplit {
  purposes: readonly { key: SunburstKey; leaf: ProductionPurpose }[];
  note: string;
  remainders?: readonly [SunburstKey, SunburstKey];
}

function purposeOf(split: PurposeSplit, purpose: ProductionPurpose | null): SunburstKey {
  const [unrecorded, other] = split.remainders ?? REMAINDER_BANDS;
  if (purpose === 'unknown') return unrecorded;
  return split.purposes.find(shown => shown.leaf === purpose)?.key ?? other;
}

const single = (leaf: ProductionSpendingLeaf): SpendingDesign =>
  ({ variant: leaf.variant, typeKey: leaf.typeKey, name: leaf.name, domain: leaf.domain, rocket: leaf.rocket, names: leaf.names, parts: [leaf] });

export function purposeItems(reply: SpendingData, leaves: readonly ProductionSpendingLeaf[], window: SpendingWindow, split: PurposeSplit): SunburstItem[] {
  const observations = reply.equipmentObservations;
  const futureEnd = window.end > window.hour;
  const finals = spendingFinals(reply, leaves.map(single), window.from, window.end, window.hour);
  const amounts = leaves.map(leaf => amountIn(leaf.runs, window.from, window.hour, observations));
  const missing = (at: 'now' | 'end', key: SunburstKey) => (at === 'end' && futureEnd) ? false : leaves.some((leaf, index) => unresolvedLeaf(leaf)
    && purposeOf(split, leaf.purpose) === key && (at === 'now' ? amounts[index].exact : finals[index].exact) !== 0n);
  const unprovenIn = (leaf: ProductionSpendingLeaf, until: number) => leaf.unproven.length > 0
    && amountIn(leaf.unproven, window.from, until, observations).exact !== 0n;
  const purposeNow = leaves.some(leaf => unprovenIn(leaf, window.hour));
  const purposeEnd = !futureEnd && leaves.some(leaf => unprovenIn(leaf, window.end));
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
      final: futureEnd ? 0 : final.geometry,
      ...(futureEnd ? { weight: value.exact == null ? 0 : Number(value.exact) } : {}),
      finalUnknown: futureEnd || final.exact == null,
      present: observed, counted: observed ? [`${leaf.variant}`] : [],
    });
  });
  const equipment = (until: number) => amountIn(reply.equipmentTotal, window.from, until, observations);
  const unreadNow = equipment(window.hour);
  const unreadEnd = futureEnd ? { exact: null, reason: null } : equipment(window.end);
  if (unreadNow.exact == null || unreadEnd.exact == null) for (const { key } of split.purposes) items.push({
    path: [key, UNREAD_KEY], value: null, exact: null, unknownValue: unreadNow.exact == null,
    note: unreadNow.exact == null ? unknownLabel(unreadNow.reason ?? 'outsideRead')
      : unreadEnd.reason ? unknownLabel(unreadEnd.reason) : undefined,
    final: 0, finalUnknown: unreadEnd.exact == null,
    present: unreadNow.exact == null,
  });
  for (const key of [...shown, purposeOf(split, 'unknown'), purposeOf(split, null)]) {
    if (!missing('now', key) && !missing('end', key)) continue;
    items.push({
      path: [key, UNRESOLVED_KEY], value: 0, exact: 0n,
      unknownLevels: [missing('now', key), true, false], finalUnknownLevels: [missing('end', key), true, false],
      note: TYPE_NOTE, final: 0, finalUnknown: futureEnd, present: missing('now', key),
    });
  }
  return items;
}

const coloured = (key: SunburstKey, colour: string | undefined): SunburstKey => colour ? { ...key, colour } : key;

export function colouredSplit(split: PurposeSplit, colours: readonly string[], skip = 0): PurposeSplit {
  const after = split.purposes.length + skip;
  return { ...split, purposes: split.purposes.map((purpose, index) => ({ ...purpose, key: coloured(purpose.key, colours[index]) })),
    remainders: [coloured(UNRECORDED, colours[after]), coloured(OTHER, colours[after + 1])] };
}

export function dockyardItems(reply: SpendingData, window: SpendingWindow, colours: readonly string[] = []): SunburstItem[] {
  return [...purposeItems(reply, domainLeaves(reply, 'naval'), window, colouredSplit(DOCKYARD_SPLIT, colours, 1)),
    ...repairItems(reply, window, coloured(PURPOSES[2], colours[2]))];
}

export const BANDS = [...PURPOSES, ...REMAINDER_BANDS];

export const repairsRead = (reply: SpendingData, atHour = Infinity) => repairsSince(reply, atHour) != null;

export function nearestInteger(numerator: bigint, denominator = 1n): bigint {
  if (denominator === 1n) return numerator;
  const half = denominator / 2n;
  return numerator < 0n ? -((-numerator + half) / denominator) : (numerator + half) / denominator;
}

export function formatExactIc(exact: bigint, denominator: bigint, decimals: number, digits: number): string {
  const shown = Math.max(0, Math.min(digits, decimals));
  return formatAmount(nearestInteger(exact * 10n ** BigInt(shown), denominator * 10n ** BigInt(decimals)), shown, shown);
}

export const reasonLabel = (reason: string | null) => reason === UNRESOLVED_REASON ? TYPE_NOTE
  : unknownLabel((reason ?? 'outsideRead') as ProductionSpendingUnknown);

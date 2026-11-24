import type { SunburstItem, SunburstKey } from '../../charts/sunburst/layout';
import type { ChartLayer } from '../../charts/TimeChart';
import { CAP_HEADROOM, columnAt, drawPreparedColumns, prepareColumn } from '../../charts/columns.ts';
import type { ColumnPeriod, PreparedColumn } from '../../charts/columns';
import type { Range } from '../../charts/view';
import { runAt } from '../../charts/series.ts';
import { UNRESOLVED_ARM, UNRESOLVED_MARK, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { firstHourOfMonth, gameDate, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import { amountIn, spendingFinals, unknownLabel, validateProductionSpendingReply } from '../production-spending/spendingData.ts';
import type { SpendingData } from '../production-spending/spendingData';
import type { SpendingAmount, SpendingDesign, SpendingWindow } from '../production-spending/spendingData';
import type { Domain, ProductionPurpose, ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun, ProductionSpendingUnknown } from '../../generated/dashboard';

export const LEVELS = ['purposeLevel', 'typeLevel', 'variantLevel'] as const;

export const PURPOSES = [
  { id: 'new', label: 'New ships' },
  { id: 'refit', label: 'Refits' },
  { id: 'repair', label: 'Repairs' },
] as const;

export const REPAIR_NOTE = 'Dockyard output used for repairs.';
const UNATTRIBUTED: SunburstKey = { id: 'unattributed', label: 'Unattributed', last: true };
export const TYPE_NOTE = 'Some designs’ types are unknown.';
const PURPOSE_NOTE = 'Whether some of it was refits was not recorded.';
const DOCKYARD_SPLIT: PurposeSplit = { purposes: [{ key: PURPOSES[0], leaf: 'new' }, { key: PURPOSES[1], leaf: 'refit' }], note: PURPOSE_NOTE };
const SPLIT_NOTE = 'Its split by design is not known here.';
export const UNRESOLVED_REASON = 'unresolved';
const UNRESOLVED_KEY: SunburstKey = { id: UNRESOLVED_ARM, label: domainName(UNRESOLVED_ARM), last: true, keepZero: true };
export const UNRECORDED: SunburstKey = { id: 'purpose-unrecorded', label: 'Purpose not recorded', last: true, keepZero: true };
export const OTHER: SunburstKey = { id: 'other-purpose', label: 'Other purposes', last: true };
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
export const exactOf = (amount: SpendingAmount): Exact => ({ exact: amount.exact, reason: amount.reason });

export const unresolvedLeaf = (leaf: ProductionSpendingLeaf) => leaf.typeKey.startsWith(UNRESOLVED_MARK);

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

export const coloured = (key: SunburstKey, colour: string | undefined): SunburstKey => colour ? { ...key, colour } : key;

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

interface ColumnSummary { low: number; high: number; any: boolean; unknown: boolean }

const summaryOf = (prepared: PreparedColumn): ColumnSummary => ({
  low: prepared.low, high: prepared.high, any: prepared.any, unknown: prepared.hasUnknown,
});

function combineSummary(a: ColumnSummary, b: ColumnSummary): ColumnSummary {
  if (!a.any) return b;
  if (!b.any) return a;
  return { low: Math.min(a.low, b.low), high: Math.max(a.high, b.high), any: true, unknown: a.unknown || b.unknown };
}

function summaryRange(summary: ColumnSummary): Range | null {
  if (!summary.any) return null;
  if (!summary.unknown) return [summary.low, summary.high];
  const span = summary.high - summary.low;
  return [summary.low, span > 0 ? summary.high + span * CAP_HEADROOM : summary.high + 1];
}

export function headMonthLayer(columns: readonly ColumnPeriod[], colours: readonly string[],
  through: (index: number, hour: number) => ColumnPeriod, headRead: () => number | null): ChartLayer {
  const basePrepared = columns.map(prepareColumn);
  const drawn = [...basePrepared];
  const summaries = basePrepared.map(summaryOf);
  let patched = -1;
  let held: { hour: number; index: number; prepared: PreparedColumn; summary: ColumnSummary } | null = null;
  let rangeCache: { start: number; end: number; skip: number; summary: ColumnSummary } | null = null;

  const firstOverlap = (x0: number): number => {
    let low = 0, high = columns.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (columns[mid].lastHour < x0) low = mid + 1;
      else high = mid;
    }
    return low;
  };

  const lastOverlap = (x1: number): number => {
    let low = 0, high = columns.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (columns[mid].firstHour <= x1) low = mid + 1;
      else high = mid;
    }
    return low - 1;
  };

  const sync = (rawHead: number | null) => {
    const head = rawHead == null ? null : Math.floor(rawHead);
    const index = head == null ? -1 : columnAt(columns, head);
    if (head == null || index < 0 || (head >= columns[index].lastHour && index < columns.length - 1)) {
      if (patched >= 0) {
        drawn[patched] = basePrepared[patched];
        patched = -1;
      }
      return null;
    }
    if (!held || held.hour !== head || held.index !== index) {
      const prepared = prepareColumn(through(index, Math.min(head, columns[index].lastHour)));
      held = { hour: head, index, prepared, summary: summaryOf(prepared) };
    }
    if (patched >= 0 && patched !== index) drawn[patched] = basePrepared[patched];
    drawn[index] = held.prepared;
    patched = index;
    return held;
  };

  return {
    draw(ctx, frame, tokens, state) {
      sync(state.playhead ?? headRead());
      drawPreparedColumns(drawn, colours, {}, ctx, frame, tokens, state);
    },
    extent(x) {
      const rawHead = headRead();
      const active = sync(rawHead);
      const until = rawHead == null ? x[1] : Math.min(x[1], Math.floor(rawHead));
      const start = firstOverlap(x[0]);
      const end = lastOverlap(until);
      if (start > end) return null;
      const skip = active && active.index >= start && active.index <= end ? active.index : -1;
      if (!rangeCache || rangeCache.start !== start || rangeCache.end !== end || rangeCache.skip !== skip) {
        let completed: ColumnSummary = { low: 0, high: 0, any: false, unknown: false };
        for (let i = start; i <= end; i++) {
          if (i === skip) continue;
          completed = combineSummary(completed, summaries[i]);
        }
        rangeCache = { start, end, skip, summary: completed };
      }
      const total = skip >= 0 && active ? combineSummary(rangeCache.summary, active.summary) : rangeCache.summary;
      return summaryRange(total);
    },
  };
}

export function monthSpansFor(first: number, last: number, limit: number, live: boolean): { firstHour: number; lastHour: number }[] {
  if (!live) return monthsOf(first, last);
  const date = gameDate(last);
  const end = date ? Math.min(limit, firstHourOfMonth(monthIndex(date) + 1) - 1) : Math.min(limit, last);
  return monthsOf(first, end);
}

export function createLiveMonthCache<O>(compute: (data: SpendingData, options: O, first: number, last: number, branchFirst: number) => PurposeMonth) {
  let cached: { data: SpendingData; optionsKey: string; first: number; last: number; branchFirst: number; month: PurposeMonth } | null = null;
  return (data: SpendingData, options: O, optionsKey: string, first: number, last: number, branchFirst: number): PurposeMonth => {
    if (cached && cached.data === data && cached.optionsKey === optionsKey && cached.first === first && cached.last === last && cached.branchFirst === branchFirst) {
      return cached.month;
    }
    const month = compute(data, options, first, last, branchFirst);
    cached = { data, optionsKey, first, last, branchFirst, month };
    return month;
  };
}

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

export interface PurposeMonth { firstHour: number; lastHour: number; bands: readonly Exact[]; total: Exact; note?: string; uncertain: readonly boolean[] }

const part = (month: SpentMonth, purposes: readonly ProductionPurpose[]): Exact => month.amounts
  ? { exact: purposes.reduce((sum, purpose) => sum + month.amounts![purpose], 0n), reason: null }
  : { exact: null, reason: month.reason };

export function dockyardMonth(reply: SpendingData, first: number, last: number, branchFirst: number): PurposeMonth {
  const spent = spentMonth(reply, domainLeaves(reply, 'naval'), first, last, branchFirst);
  const repairs = reply.repairs ? rateSum(reply.repairs.total, first, last, reply.repairs.denominator) : { exact: null, reason: 'notRecorded' };
  const bands = [part(spent, ['new']), part(spent, ['refit']), repairs, part(spent, ['unknown']), part(spent, ['converted'])];
  const purposes = spent.purposeUnproven || spent.unresolved;
  return { firstHour: first, lastHour: last, bands, total: monthTotal(spent, bands, BigInt(reply.repairs?.denominator ?? 1)),
    note: monthNote(spent), uncertain: [purposes, purposes, false, spent.purposeUnproven, false] };
}

export function visibleMonths(months: readonly PurposeMonth[], at: number | null,
  monthThrough: (index: number, hour: number) => PurposeMonth): PurposeMonth[] {
  if (at == null) return [...months];
  const head = Math.floor(at);
  const shown: PurposeMonth[] = [];
  for (let index = 0; index < months.length; index++) {
    const month = months[index];
    if (month.firstHour > head) break;
    shown.push(head < month.lastHour ? monthThrough(index, head) : month);
  }
  return shown;
}

export function shownMonthBands<T extends { id: string }>(bands: readonly T[], months: readonly PurposeMonth[]): { index: number; band: T }[] {
  const remainder = new Set<string>(REMAINDER_BANDS.map(band => band.id));
  return bands.flatMap((band, index) => !remainder.has(band.id)
    || months.some(month => { const part = month.bands[index]; return (part && (part.exact == null || part.exact !== 0n)) || month.uncertain[index] === true; })
    ? [{ index, band }] : []);
}

export function monthColumns(months: readonly PurposeMonth[], decimals: number): ColumnPeriod[] {
  return months.map(month => {
    const values = month.bands.map(band => band.exact == null ? null : Number(band.exact) / Number(band.denominator ?? 1n) / 10 ** decimals);
    const allUnknown = values.length > 0 && values.every(value => value === null);
    return { firstHour: month.firstHour, lastHour: month.lastHour, values: allUnknown ? [] : values, ...(allUnknown ? { unknown: true } : {}), uncertain: month.uncertain };
  });
}

export function exactSum(parts: readonly Exact[], denominator = 1n): Exact {
  let exact = 0n;
  for (const part of parts) {
    if (part.exact == null) return { exact: null, reason: part.reason };
    exact += part.denominator == null ? part.exact * denominator : part.exact;
  }
  return denominator === 1n ? { exact, reason: null } : { exact, reason: null, denominator };
}

export function monthTotal(month: SpentMonth, bands: readonly Exact[], denominator = 1n): Exact {
  if (month.unresolved) return { exact: null, reason: UNRESOLVED_REASON };
  return exactSum(bands, denominator);
}

export function monthLabel(hour: number): string {
  const date = gameDate(hour);
  return date ? `${monthNames[date.month - 1]} ${date.year}` : 'Unavailable date';
}

export function nearestInteger(numerator: bigint, denominator = 1n): bigint {
  if (denominator === 1n) return numerator;
  const half = denominator / 2n;
  return numerator < 0n ? -((-numerator + half) / denominator) : (numerator + half) / denominator;
}

export function formatExactIc(exact: bigint, denominator: bigint, decimals: number, digits: number): string {
  const shown = Math.max(0, Math.min(digits, decimals));
  return formatAmount(nearestInteger(exact * 10n ** BigInt(shown), denominator * 10n ** BigInt(decimals)), shown, shown);
}

export function monthNote(month: SpentMonth, purposeNote = PURPOSE_NOTE): string | undefined {
  if (month.purposeUnproven) return purposeNote;
  if (month.unresolved) return TYPE_NOTE;
  return undefined;
}

export const reasonLabel = (reason: string | null) => reason === UNRESOLVED_REASON ? TYPE_NOTE
  : unknownLabel((reason ?? 'outsideRead') as ProductionSpendingUnknown);

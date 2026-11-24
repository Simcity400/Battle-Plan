import { answersKey, isSlotKey } from '../../dashboard/countryKey.ts';
import type { BalanceHistory, CountryRef, BalanceRange, BalanceRecord, BalanceSide, HolderBalances, HourSpan, PoliticsHistoryReply, PoliticsRun, PoliticsStateReply } from '../../generated/dashboard';
import type { PoliticsStateAt } from '../../dashboard/politicsHistory';
import type { Span } from '../../charts/series';
import { runAt } from '../../charts/series.ts';
import { formatFixed, formatSigned } from '../../charts/valueAxis.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';
import { fixed5Text, integerText, isHolderTags, isPoliticsRuns, readAt, tagAt, validatePoliticsHistory, valueAt } from '../../dashboard/politicsHistory.ts';

const BALANCE_SCALE = 5;
const UNIT = 100_000;

export const BALANCE_UNAVAILABLE = 'Balance name unavailable';
export const RANGE_UNAVAILABLE = 'Range name unavailable';
export const SIDE_UNAVAILABLE = 'Side name unavailable';

type BalanceSubject = Pick<HolderBalances, 'balances' | 'shown'>;
type BalanceNames = PoliticsHistoryReply['names'];

const token = (value: unknown): value is string => typeof value === 'string' && value.length > 0;
const nullableText = (value: unknown) => value === null || typeof value === 'string';
const optionalText = (value: unknown) => value === undefined || value === null || typeof value === 'string';
const isSide = (value: unknown): value is BalanceSide => object(value) && typeof value.token === 'string' && nullableText(value.name) && optionalText(value.reason);
const bound = (value: unknown) => value === null || typeof value === 'string' && fixed5Text(value);
const isRange = (value: unknown): value is BalanceRange => object(value) && typeof value.token === 'string' && nullableText(value.name)
  && optionalText(value.reason) && bound(value.min) && bound(value.max);
const isNames = (value: unknown) => object(value) && Object.values(value).every(name => typeof name === 'string');
const tokenRun = (value: string) => value.length > 0;

function isBalanceHistory(value: unknown): value is BalanceHistory {
  return object(value) && token(value.token) && nullableText(value.name)
    && (value.left === null || isSide(value.left)) && (value.right === null || isSide(value.right))
    && Array.isArray(value.ranges) && value.ranges.every(isRange)
    && isPoliticsRuns(value.values, fixed5Text) && isPoliticsRuns(value.record, integerText)
    && isPoliticsRuns(value.range, tokenRun) && isPoliticsRuns(value.trending, tokenRun);
}

const isBalances = (value: unknown) => Array.isArray(value) && value.every(isBalanceHistory)
  && new Set(value.map(balance => (balance as BalanceHistory).token)).size === value.length;

const isHolders = (reply: Record<string, unknown>) => isNames(reply.names) && Array.isArray(reply.holderBalances)
  && reply.holderBalances.every(entry => object(entry) && token(entry.holder) && isHolderTags(entry.tags)
    && isBalances(entry.balances) && isPoliticsRuns(entry.shown, tokenRun));

export function validateCountryBalances(value: unknown): PoliticsHistoryReply {
  return validatePoliticsHistory(value, 'country', isHolders);
}

export function countryHolder(reply: Pick<PoliticsHistoryReply, 'holderBalances'> & { asked?: CountryRef | null }, tag: string, at: number): HolderBalances | null {
  const held = reply.holderBalances;
  const bySlot = isSlotKey(tag);
  if (bySlot && !answersKey(reply.asked, tag)) return null;
  const matches = (named: string | null) => named !== null && (bySlot || named === tag);
  const now = held.find(holder => matches(tagAt(holder.tags, at)));
  if (now) return now;
  let before: { holder: HolderBalances; hour: number } | null = null;
  for (const holder of held) for (const entry of holder.tags)
    if (matches(entry.tag) && entry.hour <= at && (!before || entry.hour > before.hour)) before = { holder, hour: entry.hour };
  return before?.holder ?? held.find(holder => holder.tags.some(entry => matches(entry.tag))) ?? null;
}

const nullableFixed5 = (value: unknown) => value === null || typeof value === 'string' && fixed5Text(value);
const isRecord = (value: unknown): value is BalanceRecord => object(value) && typeof value.ordinal === 'number' && Number.isSafeInteger(value.ordinal)
  && token(value.token) && nullableText(value.name) && optionalText(value.nameReason)
  && isSide(value.left) && isSide(value.right) && (value.trending === null || isSide(value.trending))
  && (value.range === null || isRange(value.range)) && Array.isArray(value.ranges) && value.ranges.every(isRange)
  && Array.isArray(value.members) && value.members.every(member => object(member) && (member.tag === null || opaqueTag(member.tag))
    && nullableFixed5(member.daily) && nullableFixed5(member.weekly))
  && (value.sources === null || Array.isArray(value.sources) && value.sources.every(source => object(source) && typeof source.token === 'string'
    && nullableText(source.name) && typeof source.daily === 'string' && fixed5Text(source.daily) && typeof source.weekly === 'string' && fixed5Text(source.weekly)));

export function validateBalanceList(value: unknown): PoliticsStateReply {
  if (!object(value) || typeof value.branchId !== 'string' || value.view !== 'balances'
    || !Array.isArray(value.balances) || !value.balances.every(isRecord)
    || !(value.current === null || Array.isArray(value.current) && value.current.every(entry => object(entry) && opaqueTag(entry.tag)
      && typeof entry.record === 'number' && Number.isSafeInteger(entry.record)))
    || !Array.isArray(value.unavailable) || !value.unavailable.every(entry => object(entry) && typeof entry.label === 'string'
      && typeof entry.reason === 'string' && typeof entry.count === 'number' && Number.isSafeInteger(entry.count))
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string'))
    throw new Error('The balance of power details reply is malformed.');
  return value as unknown as PoliticsStateReply;
}

export const balanceName = (balance: Pick<BalanceHistory, 'token' | 'name'>, names: BalanceNames): string | null =>
  balance.name ?? names[balance.token] ?? null;

export function rangeName(balance: Pick<BalanceHistory, 'ranges'>, range: string, names: BalanceNames): string | null {
  return names[range] ?? balance.ranges.find(item => item.token === range)?.name ?? null;
}

export function sideName(balance: Pick<BalanceHistory, 'left' | 'right'>, side: string, names: BalanceNames): string | null {
  return names[side] ?? [balance.left, balance.right].find(item => item?.token === side)?.name ?? null;
}

export const valueText = (raw: string | bigint) => formatFixed(raw, BALANCE_SCALE, 2);
export const exactValue = (raw: string | bigint) => formatSigned(raw, BALANCE_SCALE, BALANCE_SCALE);
export const changeText = (raw: string | bigint) => formatSigned(raw, BALANCE_SCALE, BALANCE_SCALE);
export const plotValue = (raw: string | bigint) => Number(typeof raw === 'bigint' ? raw : BigInt(raw)) / UNIT;

export function balanceAxis(balance: Pick<BalanceHistory, 'ranges'> | null): { min: number; max: number; ticks: number[] | undefined } {
  const bounds = [...new Set((balance?.ranges ?? []).flatMap(range => [range.min, range.max]).filter((bound): bound is string => bound !== null))]
    .map(plotValue).sort((a, b) => a - b);
  return { min: Math.min(-1, bounds[0] ?? -1), max: Math.max(1, bounds.at(-1) ?? 1), ticks: bounds.length ? bounds : undefined };
}

export interface BalanceAt { value: string | null; range: string | null; trending: string | null }
export function balanceAt(balance: BalanceHistory, hour: number): BalanceAt {
  return { value: valueAt(balance.values, hour), range: valueAt(balance.range, hour), trending: valueAt(balance.trending, hour) };
}

function visibleBalances(subject: BalanceSubject, limit: number): BalanceHistory[] {
  return subject.balances.filter(balance => balance.values.length && balance.values[0].firstHour <= limit)
    .map((balance, index) => ({ balance, index }))
    .sort((a, b) => a.balance.values[0].firstHour - b.balance.values[0].firstHour || a.index - b.index)
    .map(item => item.balance);
}

export interface BalanceChoice {
  balance: BalanceHistory | null;
  automatic: boolean;
  missing: string | null;
}

export function chooseBalance(subject: BalanceSubject, saved: string, at: number, limit: number): BalanceChoice {
  const visible = visibleBalances(subject, limit);
  const byToken = new Map(visible.map(balance => [balance.token, balance]));
  if (saved && byToken.has(saved)) return { balance: byToken.get(saved)!, automatic: false, missing: null };
  const missing = saved ? saved : null;
  const shown = (run: PoliticsRun | null | undefined) => run && byToken.has(run.value) ? byToken.get(run.value)! : null;
  const here = at <= limit ? shown(runAt(subject.shown, at)) : null;
  const before = here ?? shown([...subject.shown].reverse().find(run => run.firstHour <= Math.min(at, limit)));
  const after = before ?? shown(subject.shown.find(run => run.firstHour <= limit));
  return { balance: after ?? visible[0] ?? null, automatic: true, missing };
}

export type BalanceAbsence = { kind: 'unread' } | { kind: 'none'; recorded: boolean };
export function balanceAbsence(known: readonly HourSpan[], at: number, limit: number, unknown: readonly Span[]): BalanceAbsence {
  if (!readAt(known, at)) return { kind: 'unread' };
  return { kind: 'none', recorded: unknown.some(span => span.firstHour <= limit) };
}

interface BalanceMemberRef { tag: string }
interface BalanceRow {
  key: string;
  token: string;
  name: string | null;
  range: BalanceRange | null;
  trending: BalanceSide | null;
  members: BalanceMemberRef[];
}
interface BalanceRowsView {
  rows: BalanceRow[];
  untagged: number;
  membership: string | null;
}

export function balanceRows(reply: Pick<PoliticsStateAt, 'balances' | 'hour'>, matches: (tag: string, hour: number) => boolean,
  unrecorded: (hour: number) => string | null = () => null): BalanceRowsView {
  const rows: BalanceRow[] = [];
  let untagged = 0, membership: string | null = null;
  for (const record of reply.balances) {
    const members = record.members.flatMap(member => member.tag === null ? [] : [{ tag: member.tag }]);
    untagged += record.members.length - members.length;
    if (!members.some(member => matches(member.tag, reply.hour))) { membership ??= unrecorded(reply.hour); continue; }
    rows.push({ key: String(record.ordinal), token: record.token, name: record.name, range: record.range, trending: record.trending, members });
  }
  return { rows, untagged, membership };
}

export function recordFor(reply: Pick<PoliticsStateAt, 'balances' | 'current'>, token: string | null, tag: string): BalanceRecord | null {
  const current = reply.current?.find(entry => entry.tag === tag);
  const own = current ? reply.balances.find(record => record.ordinal === current.record) ?? null : null;
  if (own && (token === null || own.token === token)) return own;
  if (token === null) return null;
  return reply.balances.find(record => record.token === token && record.members.some(member => member.tag === tag)) ?? null;
}

export function holderRecord(reply: Pick<PoliticsStateAt, 'balances' | 'current' | 'hour'>, holder: Pick<HolderBalances, 'tags'>,
  token: string | null): BalanceRecord | null {
  const tag = tagAt(holder.tags, reply.hour);
  return tag === null ? null : recordFor(reply, token, tag);
}

export function sumWeeklyDrift(record: Pick<BalanceRecord, 'members'>): bigint | null {
  if (!record.members.length) return null;
  let total = 0n;
  for (const member of record.members) {
    if (member.weekly === null) return null;
    total += BigInt(member.weekly);
  }
  return total;
}

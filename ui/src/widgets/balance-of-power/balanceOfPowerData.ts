import type { BalanceHistory, BalanceRange, BalanceRecord, BalanceSide, PoliticsHistoryReply, PoliticsStateReply } from '../../generated/dashboard';
import { formatFixed, formatSigned } from '../../charts/valueAxis.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';
import { fixed5Text, integerAt, integerText, isHolderTags, isPoliticsRuns, validatePoliticsHistory } from '../../dashboard/politicsHistory.ts';

const BALANCE_SCALE = 5;
const UNIT = 100_000;

export const BALANCE_UNAVAILABLE = 'Balance name unavailable';
export const RANGE_UNAVAILABLE = 'Range name unavailable';

const token = (value: unknown): value is string => typeof value === 'string' && value.length > 0;
const nullableText = (value: unknown) => value === null || typeof value === 'string';
const optionalText = (value: unknown) => value === undefined || value === null || typeof value === 'string';
const isSide = (value: unknown): value is BalanceSide => object(value) && typeof value.token === 'string' && nullableText(value.name) && optionalText(value.reason);
const bound = (value: unknown) => value === null || typeof value === 'string' && fixed5Text(value);
const isRange = (value: unknown): value is BalanceRange => object(value) && typeof value.token === 'string' && nullableText(value.name)
  && optionalText(value.reason) && bound(value.min) && bound(value.max);
const isNames = (value: unknown) => object(value) && Object.values(value).every(name => typeof name === 'string');
const tokenRun = (value: string) => value.length > 0;
const hourOrNull = (value: unknown) => value === null || (typeof value === 'number' && Number.isSafeInteger(value));

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
  && reply.holderBalances.length <= 1
  && reply.holderBalances.every(entry => object(entry) && token(entry.holder) && isHolderTags(entry.tags)
    && isBalances(entry.balances) && isPoliticsRuns(entry.shown, tokenRun))
  && typeof reply.hour === 'number' && Number.isSafeInteger(reply.hour) && hourOrNull(reply.nextChange)
  && (reply.nextChange === null || (reply.nextChange as number) > (reply.hour as number))
  && nullableText(reply.unread) && nullableText(reply.selected)
  && Array.isArray(reply.known) && reply.known.every(span => object(span) && (span.lastHour as number) <= (reply.hour as number));

export function validateCountryBalances(value: unknown): PoliticsHistoryReply {
  return validatePoliticsHistory(value, 'country', isHolders);
}

export function balanceHistoryProof(data: unknown, _requested: readonly [number, number], hour: number): readonly [number, number] | null {
  const reply = data as PoliticsHistoryReply;
  if (reply.hour === null) return null;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= hour && hour <= last ? [reply.hour, last] : null;
}

const isChange = (value: unknown) => value === null || (object(value) && typeof value.daily === 'string' && fixed5Text(value.daily)
  && typeof value.weekly === 'string' && fixed5Text(value.weekly) && typeof value.weeklyChange === 'string' && fixed5Text(value.weeklyChange)
  && (value.shown === 'daily' || value.shown === 'weekly') && typeof value.rate === 'string' && fixed5Text(value.rate));

const nullableFixed5 = (value: unknown) => value === null || typeof value === 'string' && fixed5Text(value);
const isRecord = (value: unknown): value is BalanceRecord => object(value) && typeof value.ordinal === 'number' && Number.isSafeInteger(value.ordinal)
  && token(value.token) && nullableText(value.name) && optionalText(value.nameReason)
  && isSide(value.left) && isSide(value.right) && (value.trending === null || isSide(value.trending))
  && (value.range === null || isRange(value.range)) && Array.isArray(value.ranges) && value.ranges.every(isRange)
  && Array.isArray(value.members) && value.members.every(member => object(member) && (member.tag === null || opaqueTag(member.tag))
    && nullableFixed5(member.daily) && nullableFixed5(member.weekly))
  && isChange(value.change)
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

export function selectedBalance(reply: PoliticsHistoryReply): BalanceHistory | null {
  const holder = reply.holderBalances[0];
  return holder && reply.selected !== null ? holder.balances.find(balance => balance.token === reply.selected) ?? null : null;
}

export function recordAt(reply: PoliticsStateReply | null | undefined, balance: BalanceHistory, hour: number): BalanceRecord | null {
  const ordinal = integerAt(balance.record, hour);
  if (!reply || ordinal === null) return null;
  return reply.balances.find(record => record.ordinal === ordinal && record.token === balance.token) ?? null;
}

export const balanceName = (balance: Pick<BalanceHistory, 'token' | 'name'>, names: PoliticsHistoryReply['names']): string | null =>
  balance.name ?? names[balance.token] ?? null;

export const valueText = (raw: string | bigint) => formatFixed(raw, BALANCE_SCALE, 2);
export const exactValue = (raw: string | bigint) => formatSigned(raw, BALANCE_SCALE, BALANCE_SCALE);
export const changeText = (raw: string | bigint) => formatSigned(raw, BALANCE_SCALE, BALANCE_SCALE);
export const plotValue = (raw: string | bigint) => Number(typeof raw === 'bigint' ? raw : BigInt(raw)) / UNIT;

export function balanceAxis(balance: Pick<BalanceHistory, 'ranges'> | null): { min: number; max: number; ticks: number[] | undefined } {
  const bounds = [...new Set((balance?.ranges ?? []).flatMap(range => [range.min, range.max]).filter((bound): bound is string => bound !== null))]
    .map(plotValue).sort((a, b) => a - b);
  return { min: Math.min(-1, bounds[0] ?? -1), max: Math.max(1, bounds.at(-1) ?? 1), ticks: bounds.length ? bounds : undefined };
}

export function sideLabel(side: BalanceSide | null | undefined): { readonly label: string; readonly reason: string | null } | null {
  if (!side) return null;
  if (side.name) return { label: side.name, reason: null };
  return { label: 'Side name not recorded', reason: side.reason ?? 'The recorded localisation has no name for this side' };
}

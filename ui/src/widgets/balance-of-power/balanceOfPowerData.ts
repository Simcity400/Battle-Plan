import type { GatedTime } from '../../dashboard/regionTime';
import type { PinnedSpec } from '../../charts/drill/drillState';
import { answersKey, isSlotKey } from '../../dashboard/countryKey.ts';
import type { BalanceHistory, CountryRef, BalanceRange, BalanceRecord, BalanceSide, HolderBalances, HourSpan, PoliticsHistoryReply, PoliticsRun, PoliticsStateReply } from '../../generated/dashboard';
import type { PoliticsStateAt } from '../../dashboard/politicsHistory';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { DrillPart } from '../../charts/drill/parts';
import type { Run, Span } from '../../charts/series';
import { runAt } from '../../charts/series.ts';
import type { SortValue } from '../../charts/table/logic';
import { dayLabel } from '../../charts/draw.ts';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { WEEKLY_BREAKDOWN } from './definition.ts';
import { runState } from '../../charts/timeline/runState.ts';
import { formatFixed, formatSigned } from '../../charts/valueAxis.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';
import { fixed5Text, integerText, isHolderTags, isPoliticsRuns, readAt, tagAt, validatePoliticsHistory, valueAt } from '../../dashboard/politicsHistory.ts';

export const BALANCE_SCALE = 5;
const UNIT = 100_000;

export const BALANCE_UNAVAILABLE = 'Balance name unavailable';
export const RANGE_UNAVAILABLE = 'Range name unavailable';
export const SIDE_UNAVAILABLE = 'Name not recorded (unresolved localisation)';
export const SOURCE_UNAVAILABLE = 'Modifier name unavailable';
export const COUNTRY_UNAVAILABLE = 'Country unavailable';

export type BalanceSubject = Pick<HolderBalances, 'balances' | 'shown'>;
export type BalanceNames = PoliticsHistoryReply['names'];


const token = (value: unknown): value is string => typeof value === 'string' && value.length > 0;
const nullableText = (value: unknown) => value === null || typeof value === 'string';
const isSide = (value: unknown): value is BalanceSide => object(value) && typeof value.token === 'string' && nullableText(value.name);
const bound = (value: unknown) => value === null || typeof value === 'string' && fixed5Text(value);
const isRange = (value: unknown): value is BalanceRange => object(value) && typeof value.token === 'string' && nullableText(value.name)
  && bound(value.min) && bound(value.max);
const isNames = (value: unknown) => object(value) && Object.values(value).every(name => typeof name === 'string');
const tokenRun = (value: string) => value.length > 0;

export function isBalanceHistory(value: unknown): value is BalanceHistory {
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

export function validateMemberships(value: unknown): PoliticsHistoryReply {
  return validatePoliticsHistory(value, 'memberships', isHolders);
}

export function memberSince(reply: Pick<PoliticsHistoryReply, 'holderBalances'>, tag: string, token: string, hour: number): number | null {
  const holder = reply.holderBalances.find(entry => tagAt(entry.tags, hour) === tag);
  const balance = holder?.balances.find(entry => entry.token === token);
  if (!balance) return null;
  const span = lifetimeSpans({ values: balance.record }, hour).at(-1);
  return span && span.lastHour === hour ? span.firstHour : null;
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
  && token(value.token) && nullableText(value.name)
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

export function recordValue(values: LiveValues<number>, record: Pick<BalanceRecord, 'ordinal'>): bigint | null {
  const value = values.get(liveKey(String(record.ordinal), 'value'));
  return value === undefined ? null : BigInt(Math.trunc(value));
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

export function lifetimeSpans(balance: Pick<BalanceHistory, 'values'>, limit = Number.POSITIVE_INFINITY): Span[] {
  const spans: Span[] = [];
  for (const run of balance.values) {
    if (run.firstHour > limit) break;
    const lastHour = Math.min(run.lastHour, limit);
    const last = spans.at(-1);
    if (last && last.lastHour + 1 === run.firstHour) last.lastHour = lastHour;
    else spans.push({ firstHour: run.firstHour, lastHour });
  }
  return spans;
}

export function visibleBalances(subject: BalanceSubject, limit: number): BalanceHistory[] {
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

export type BalanceStatus =
  | { kind: 'active'; value: string; range: string | null; trending: string | null }
  | { kind: 'starts'; hour: number }
  | { kind: 'ended'; hour: number; by: boolean; range: string | null }
  | { kind: 'last-recorded'; hour: number; range: string | null }
  | { kind: 'unknown' };

export function balanceStatusAt(balance: BalanceHistory, at: number, limit: number, unknown: readonly Span[]): BalanceStatus {
  const now = balanceAt(balance, at);
  if (now.value !== null && at <= limit) return { kind: 'active', value: now.value, range: now.range, trending: now.trending };
  const spans = lifetimeSpans(balance, limit);
  const state = runState(spans, at, unknown);
  switch (state.kind) {
    case 'held':
    case 'none': {
      const next = spans.find(span => span.firstHour > at);
      return next ? { kind: 'starts', hour: next.firstHour } : { kind: 'unknown' };
    }
    case 'ended': {
      const last = state.lastSeen ?? state.at - 1;
      return { kind: 'ended', hour: state.at, by: state.lastSeen != null, range: valueAt(balance.range, last) };
    }
    case 'unknown': return { kind: 'last-recorded', hour: state.lastSeen, range: valueAt(balance.range, state.lastSeen) };
  }
}

export function statusText(status: BalanceStatus, balance: BalanceHistory, names: BalanceNames): string {
  const range = (token: string | null) => token === null ? null : rangeName(balance, token, names) ?? RANGE_UNAVAILABLE;
  const inRange = (token: string | null) => token === null ? '' : ` in ${range(token)}`;
  switch (status.kind) {
    case 'active': return [range(status.range), valueText(status.value),
      status.trending === null ? null : `toward ${sideName(balance, status.trending, names) ?? SIDE_UNAVAILABLE}`].filter(Boolean).join(' · ');
    case 'starts': return `Starts ${dayLabel(status.hour)}`;
    case 'ended': return `${status.by ? 'Ended by' : 'Ended'} ${dayLabel(status.hour)}${inRange(status.range)}`;
    case 'last-recorded': return `Last recorded ${dayLabel(status.hour)}${inRange(status.range)}`;
    case 'unknown': return 'Not recorded at this date';
  }
}

export type BalanceAbsence = { kind: 'unread' } | { kind: 'none'; recorded: boolean };
export function balanceAbsence(known: readonly HourSpan[], at: number, limit: number, unknown: readonly Span[]): BalanceAbsence {
  if (!readAt(known, at)) return { kind: 'unread' };
  return { kind: 'none', recorded: unknown.some(span => span.firstHour <= limit) };
}

export function plotRuns(balance: Pick<BalanceHistory, 'values'> | null): Run[] {
  return balance ? balance.values.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: plotValue(run.value) })) : [];
}

export interface BalanceLane { token: string; name: string | null; spans: Span[]; cut: boolean }

export function balanceLanes(subject: BalanceSubject, names: BalanceNames, limit: number, spanLimit = limit): BalanceLane[] {
  return visibleBalances(subject, limit).map(balance => {
    const spans = lifetimeSpans(balance, spanLimit);
    return { token: balance.token, name: balanceName(balance, names), spans, cut: (spans.at(-1)?.lastHour ?? spanLimit) < (balance.values.at(-1)?.lastHour ?? spanLimit) };
  });
}

export function pinBalanceDrill(source: WidgetSpec, track: Pick<BoundTrack, 'id'>, time: GatedTime, token: string): PinnedSpec {
  const pinned = pinnedBreakdown(source, WEEKLY_BREAKDOWN, track.id, time);
  return { ...pinned, options: { ...pinned.options, balance: token } };
}


export interface BalanceMemberRef { tag: string }
export interface BalanceRow {
  key: string;
  token: string;
  name: string | null;
  range: BalanceRange | null;
  trending: BalanceSide | null;
  members: BalanceMemberRef[];
}
export interface BalanceRowsView {
  rows: BalanceRow[]; untagged: number;
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

export const balanceStructure = (reply: PoliticsStateReply): string =>
  JSON.stringify([reply.balances.map(record => [record.ordinal, record.token, record.members.map(member => member.tag),
    record.range?.token ?? null, record.trending?.token ?? null]), reply.unavailable]);

export function balanceTableValues(values: LiveValues<number>): LiveValues<SortValue> {
  return {
    get: key => { const value = values.get(key); return value === undefined ? undefined : Math.abs(value); },
    missing: key => values.missing(key),
  };
}

export function rowHolder(row: BalanceRow, hour: number, matches: (tag: string, hour: number) => boolean): BalanceMemberRef {
  return row.members.find(member => matches(member.tag, hour)) ?? row.members[0];
}


export function recordFor(reply: Pick<PoliticsStateAt, 'balances' | 'current'>, token: string | null, tag: string): BalanceRecord | null {
  const current = reply.current?.find(entry => entry.tag === tag);
  const own = current ? reply.balances.find(record => record.ordinal === current.record) ?? null : null;
  if (own && (token === null || own.token === token)) return own;
  if (token === null) return null;
  return reply.balances.find(record => record.token === token && record.members.some(member => member.tag === tag)) ?? null;
}

export const BALANCES_NOT_READ = 'balances of power';

export type WeeklyState = { kind: 'unread' } | { kind: 'record'; record: BalanceRecord } | { kind: 'none' };
export function weeklyState(history: Pick<PoliticsHistoryReply, 'holderBalances'>, tag: string, at: number,
  detail: Pick<PoliticsStateAt, 'balances' | 'current' | 'hour' | 'unavailable'>, token: string | null): WeeklyState {
  if (detail.unavailable.some(entry => entry.label === BALANCES_NOT_READ)) return { kind: 'unread' };
  const holder = countryHolder(history, tag, at);
  const record = holder ? holderRecord(detail, holder, token) : null;
  return record ? { kind: 'record', record } : { kind: 'none' };
}

export function holderRecord(reply: Pick<PoliticsStateAt, 'balances' | 'current' | 'hour'>, holder: Pick<HolderBalances, 'tags'>,
  token: string | null): BalanceRecord | null {
  const tag = tagAt(holder.tags, reply.hour);
  return tag === null ? null : recordFor(reply, token, tag);
}

export interface DriftParts {
  sources: DrillPart[] | null;
  sourced: bigint | null;
  members: DrillPart[];
  weekly: bigint | null;
}

type MemberLabel = { nameFailure?: Error; name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean };
const sum = (values: readonly (string | null)[]): bigint | null => values.some(value => value === null) ? null : values.reduce((total, value) => total + BigInt(value!), 0n);

export function driftParts(record: BalanceRecord, member: (tag: string) => MemberLabel): DriftParts {
  const sources = record.sources?.map((source, index): DrillPart => {
    const raw = BigInt(source.weekly);
    return { id: `source:${index}`, label: source.name ?? SOURCE_UNAVAILABLE, value: plotValue(raw), raw, text: changeText(raw) };
  }) ?? null;
  const members = record.members.map((entry, index): DrillPart => {
    const label = entry.tag === null ? { name: COUNTRY_UNAVAILABLE } : member(entry.tag);
    const raw = entry.weekly === null ? null : BigInt(entry.weekly);
    return { id: `member:${index}`, label: label.name, value: raw === null ? null : plotValue(raw), raw, text: raw === null ? 'Not recorded' : changeText(raw),
      shareText: entry.daily === null ? 'Daily not recorded' : `${changeText(entry.daily)} daily`,
      ...(entry.tag === null ? {} : { countryFlag: { ...(label.nameFailure ? { nameFailure: label.nameFailure } : {}), flag: label.flag, flagReason: label.flagReason, ...(label.flagDetail !== undefined ? { flagDetail: label.flagDetail } : {}), flagPending: label.flagPending } }) };
  });
  const weekly = record.members.length ? sum(record.members.map(entry => entry.weekly)) : null;
  const sourced = record.sources ? sum(record.sources.map(source => source.weekly)) : null;
  return { sources, sourced, members, weekly };
}


export function nameWarnings(balances: readonly (Pick<BalanceHistory, 'token' | 'name'> & Partial<Pick<BalanceHistory, 'range' | 'ranges'>>)[], names: BalanceNames,
  hourRanges: readonly string[] = []): string[] {
  const missingBalances = balances.filter(balance => balanceName(balance, names) === null).length;
  const rangeTokens = new Set<string>(hourRanges);
  for (const balance of balances) for (const run of balance.range ?? [])
    if (rangeName({ ranges: balance.ranges ?? [] }, run.value, names) === null) rangeTokens.add(`${balance.token}:${run.value}`);
  return [
    ...missingBalances ? [`${missingBalances} balance name${missingBalances === 1 ? '' : 's'} unavailable`] : [],
    ...rangeTokens.size ? [`${rangeTokens.size} range name${rangeTokens.size === 1 ? '' : 's'} unavailable`] : [],
  ];
}

export function detailWarnings(reply: Pick<PoliticsStateAt, 'unavailable' | 'warnings' | 'balances'>, record: BalanceRecord | null): string[] {
  return [...reply.warnings, ...reply.unavailable.map(entry => `${entry.count} ${entry.label} unavailable: ${entry.reason}`),
    ...record && record.sources === null ? ['1 balance drift source list unavailable'] : []];
}

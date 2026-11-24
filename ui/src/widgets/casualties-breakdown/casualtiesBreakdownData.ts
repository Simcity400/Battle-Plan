import { liveKey } from '../../charts/live.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { isOrderedSpans, runAt, spansCovered } from '../../charts/series.ts';
import { CATEGORY_LIMIT } from '../../charts/palette.ts';
import { stackEdges } from '../../charts/stack.ts';
import type { StackModel } from '../../charts/stack';
import type { SunburstItem } from '../../charts/sunburst/Sunburst';
import { int, object, opaqueTag, signed32, signed64, unsigned64 } from '../../dashboard/validation.ts';
import type { HourSpan } from '../../generated/dashboard';
import type { CasualtyBreakdownReply } from '../../generated/dashboard';

const text = (value: unknown): value is string => typeof value === 'string';
const maybeText = (value: unknown) => value === null || text(value);
const spans = (value: unknown): value is HourSpan[] => isOrderedSpans(value)
  && value.every(run => signed32(run.firstHour) && signed32(run.lastHour));
const span = (value: unknown) => object(value) && signed32(value.firstHour) && signed32(value.lastHour) && value.firstHour <= value.lastHour;

export function validateBreakdown(value: unknown): CasualtyBreakdownReply {
  const bad = () => { throw new Error('The casualty breakdown reply is malformed.'); };
  if (!object(value) || !unsigned64(value.branchId) || !opaqueTag(value.tag) || typeof value.available !== 'boolean'
    || !spans(value.windows) || !(value.window === null || int(value.window) && value.window >= 0 && value.window < value.windows.length)
    || !spans(value.observed) || !Array.isArray(value.warnings) || !value.warnings.every(text)
    || typeof value.hasGap !== 'boolean' || !spans(value.countries) || !Array.isArray(value.series) || !Array.isArray(value.wars)) return bad();
  const reply = value as unknown as CasualtyBreakdownReply;
  const selected = reply.window === null ? reply.windows : [reply.windows[reply.window]];
  if (!spansCovered(reply.observed, selected) || !spansCovered(reply.countries, reply.observed)
    || !reply.countries.every(run => object(run) && text(run.identity) && run.identity.length > 0 && typeof run.incomplete === 'boolean')) return bad();
  const wars = new Set<string>();
  for (const war of reply.wars) {
    if (!span(war) || !text(war.id) || wars.has(war.id) || typeof war.startedBy !== 'boolean' || typeof war.endedBy !== 'boolean'
      || !spans(war.names) || !war.names.every(run => object(run.name) && text(run.name.text)
        && typeof run.name.known === 'boolean' && maybeText(run.name.reason) && maybeText(run.name.key))) return bad();
    wars.add(war.id);
  }
  const ids = new Set<string>();
  for (const series of reply.series) {
    if (!object(series) || !text(series.id) || ids.has(series.id) || !opaqueTag(series.enemyTag)
      || !text(series.enemyIdentity) || !text(series.countryIdentity) || !spans(series.runs)
      || !spansCovered(series.runs, reply.observed)) return bad();
    ids.add(series.id);
    for (const run of series.runs) {
      const holder = runAt(reply.countries, run.firstHour);
      if (!holder || holder.identity !== series.countryIdentity || holder.lastHour < run.lastHour
        || !signed64(run.value) || !signed32(run.readHour) || run.readHour > run.firstHour || typeof run.hourly !== 'boolean'
        || !signed32(run.groupingHour) || run.groupingHour > run.firstHour || typeof run.groupingHourly !== 'boolean'
        || !signed32(run.enemyHour) || run.enemyHour > run.firstHour || typeof run.enemyHourly !== 'boolean'
        || typeof run.incomplete !== 'boolean' || !maybeText(run.reason)
        || !(run.warId === null || text(run.warId) && wars.has(run.warId))
        || (run.warId === null) !== (run.reason !== null)) return bad();
    }
  }
  if (reply.window === null && (reply.series.length || reply.countries.length || reply.wars.length)) return bad();
  return reply;
}

export interface LossRow {
  id: string; enemyTag: string; enemyIdentity: string; enemyHour: number;
  warId: string | null; war: string; reason: string | null;
  value: bigint; readHour: number; groupingHour: number; incomplete: boolean;
}
export interface LossSnapshot { identity: string; rows: LossRow[]; total: bigint; incomplete: boolean }

export function lossesAt(reply: CasualtyBreakdownReply, hour: number): LossSnapshot | null {
  const at = Math.floor(hour), country = runAt(reply.countries, at);
  if (!runAt(reply.observed, at) || !country) return null;
  const wars = new Map(reply.wars.map(war => [war.id, war]));
  const rows: LossRow[] = [];
  for (const series of reply.series) {
    const run = runAt(series.runs, at);
    if (!run || series.countryIdentity !== country.identity) continue;
    const groupingHour = run.groupingHourly ? at : run.groupingHour;
    const war = run.warId === null ? null : wars.get(run.warId);
    const name = war ? runAt(war.names, groupingHour)?.name : null;
    rows.push({ id: series.id, enemyTag: series.enemyTag, enemyIdentity: series.enemyIdentity,
      enemyHour: run.enemyHourly ? at : run.enemyHour,
      warId: run.warId, war: run.warId === null ? 'Other' : name?.text ?? 'Not recorded',
      reason: run.reason,
      value: BigInt(run.value), readHour: run.hourly ? at : run.readHour, groupingHour, incomplete: country.incomplete || run.incomplete });
  }
  if (!rows.length && !country.incomplete) return null;
  return { identity: country.identity, rows, total: rows.reduce((sum, row) => sum + row.value, 0n),
    incomplete: country.incomplete || rows.some(row => row.incomplete) };
}
export const exactLosses = (value: bigint) => usNumber.format(value);
export const lossText = (row: Pick<LossRow, 'value' | 'incomplete'>) => `${row.incomplete ? 'Recorded subtotal: ' : ''}${row.value < 0n ? 'Correction: ' : ''}${exactLosses(row.value)}`;
export function lossDrillAt(reply: CasualtyBreakdownReply | undefined, hour: number): { rows: string; values: Map<string, bigint>; incomplete: Set<string>; total: string } {
  const now = reply ? lossesAt(reply, hour) : null;
  const values = new Map<string, bigint>(), incomplete = new Set<string>();
  for (const row of now?.rows ?? []) {
    values.set(liveKey(row.id, 'losses'), row.value);
    if (row.incomplete) incomplete.add(row.id);
  }
  return { rows: (now?.rows ?? []).map(row => `${row.id}${row.incomplete ? '!' : ''}`).sort().join(','), values, incomplete,
    total: now ? `${now.incomplete ? 'Recorded subtotal' : 'Losses taken'}: ${exactLosses(now.total)}` : '' };
}
export const warKey = (row: Pick<LossRow, 'warId'>) => row.warId ?? '\u0001unidentified';

export function lossItems(now: LossSnapshot | null, end: LossSnapshot | null,
  enemyName: (tag: string, hour: number) => string): SunburstItem[] {
  if (now && end?.identity !== now.identity) end = null;
  const items = new Map<string, SunburstItem>();
  const add = (row: LossRow, final: boolean) => {
    const key = `${warKey(row)}\0${row.enemyIdentity}`;
    const value = row.value > 0n ? row.value : 0n;
    let item = items.get(key);
    if (!item) {
      item = { path: [{ id: warKey(row), label: row.war }, { id: row.enemyIdentity, label: enemyName(row.enemyTag, row.enemyHour) }],
        value: 0, exact: 0n, final: 0, present: false, finalUnknown: end == null || end.incomplete };
      items.set(key, item);
    }
    if (final) item.final = (item.final ?? 0) + Number(value);
    else { item.value = (item.value ?? 0) + Number(value); item.exact = (item.exact ?? 0n) + value; item.present = true; }
  };
  for (const row of now?.rows ?? []) add(row, false);
  for (const row of end?.rows ?? []) add(row, true);
  return [...items.values()];
}

export function breakdownWindows(reply: CasualtyBreakdownReply | null | undefined, hour: number, following: boolean): number[] {
  if (!reply) return [];
  const index = reply.windows.findIndex(span => span.firstHour <= hour && hour <= span.lastHour);
  if (index < 0) return [];
  return [index, ...(following && index + 1 < reply.windows.length ? [index + 1] : [])];
}

export const viewWindows = (windows: readonly HourSpan[], range: readonly [number, number] | null) =>
  windows.flatMap((span, index) => !range || span.firstHour <= range[1] && range[0] <= span.lastHour ? [index] : []);
export const viewChange = (windows: readonly HourSpan[], range: readonly [number, number]) => (before: readonly [number, number] | null) =>
  before && `${viewWindows(windows, before)}` === `${viewWindows(windows, range)}` ? before : range;

export function lossStack(replies: readonly CasualtyBreakdownReply[]): { model: StackModel; keys: string[] } {
  const weights = new Map<string, bigint>();
  for (const reply of replies) for (const series of reply.series) for (const run of series.runs) {
    const value = BigInt(run.value), key = warKey(run);
    if (value > 0n) weights.set(key, (weights.get(key) ?? 0n) + value * BigInt(run.lastHour - run.firstHour + 1));
  }
  const order = [...weights].sort(([a, x], [b, y]) => x === y ? a < b ? -1 : a > b ? 1 : 0 : x > y ? -1 : 1).map(([key]) => key);
  const folded = order.length > CATEGORY_LIMIT;
  const keys = folded ? [...order.slice(0, CATEGORY_LIMIT - 1), '\u0001other'] : order;
  const indices = new Map(keys.map((key, index) => [key, index]));
  const events = new Map<number, [number, bigint][]>();
  const add = (hour: number, index: number, value: bigint) => {
    let at = events.get(hour); if (!at) { at = []; events.set(hour, at); } at.push([index, value]);
  };
  const countries = replies.flatMap(reply => reply.countries).sort((a, b) => a.firstHour - b.firstHour);
  for (const country of countries) { if (!events.has(country.firstHour)) events.set(country.firstHour, []); if (!events.has(country.lastHour + 1)) events.set(country.lastHour + 1, []); }
  for (const reply of replies) for (const series of reply.series) for (const run of series.runs) {
    const value = BigInt(run.value), index = indices.get(warKey(run)) ?? (folded ? keys.length - 1 : undefined);
    if (value <= 0n || index === undefined) continue;
    add(run.firstHour, index, value); add(run.lastHour + 1, index, -value);
  }
  const model: StackModel = { hours: [...events.keys()].sort((a, b) => a - b), edges: [], breakBefore: [] };
  const totals = keys.map(() => 0n);
  let previous: string | null = null;
  for (const hour of model.hours) {
    for (const [index, value] of events.get(hour)!) totals[index] += value;
    const country = runAt(countries, hour);
    model.edges.push(country ? stackEdges(totals.map(Number)) : null);
    model.breakBefore.push(previous !== null && country?.identity !== previous);
    previous = country?.identity ?? null;
  }
  return { model, keys };
}

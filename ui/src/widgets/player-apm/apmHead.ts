import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { SortValue } from '../../charts/table/logic';
import { CELL, cellState, cellValue, rowName, textAt } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer, FollowedRows } from '../../dashboard/follow/message';
import type { ApmGroup, ApmMembership, ApmSeries, ApmValue } from '../../generated/dashboard';

const APM = { apm: 0, confirmedApm: 1, orders: 2, pausedOrders: 3, minutes: 4, complete: 5, gaps: 6, tag: 7, hour: 8, name: 9, nameMissing: 10 } as const;
const CLOCK = { recorded: 0, playing: 1, paused: 2, complete: 3 } as const;

const bits = new DataView(new ArrayBuffer(8));
function float(answer: FollowedRows, at: number): number | null {
  if (cellState(answer, at) !== CELL.value) return null;
  bits.setBigInt64(0, cellValue(answer, at), true);
  return bits.getFloat64(0, true);
}
const whole = (answer: FollowedRows, at: number): bigint | null => cellState(answer, at) === CELL.value ? cellValue(answer, at) : null;
const textCell = (answer: FollowedRows, at: number): string | null =>
  cellState(answer, at) === CELL.value ? textAt(answer, Number(cellValue(answer, at))) : null;

export function apmHeadQuery(params: { branchId: string; group: ApmGroup; tags: string[] | null; membership: ApmMembership[] | null; players: string[] },
  branch: { firstHour: number; lastHour: number }, windowHours: number) {
  return { head: 'player-apm', params: { ...params, firstHour: branch.firstHour, lastHour: branch.lastHour, windowHours } };
}

export const clockHeadQuery = (branch: { id: string; firstHour: number; lastHour: number }) =>
  ({ head: 'campaign-clock', params: { branchId: branch.id, firstHour: branch.firstHour, lastHour: branch.lastHour } });

const rowsOf = (answer: FollowedAnswer | undefined): FollowedRows | null => answer && answer.kind !== 'notRecorded' ? answer : null;

function valueAt(answer: FollowedRows, row: number): ApmValue {
  const at = row * answer.columns;
  const count = (column: number) => whole(answer, at + column)?.toString() ?? null;
  return { apm: float(answer, at + APM.apm), confirmedApm: float(answer, at + APM.confirmedApm), orders: count(APM.orders), pausedOrders: count(APM.pausedOrders),
    unpausedMinutes: float(answer, at + APM.minutes), complete: whole(answer, at + APM.complete) === 1n, gaps: whole(answer, at + APM.gaps) === 1n };
}

export function apmHeadRows(answer: FollowedAnswer | undefined): ApmSeries[] {
  const held = rowsOf(answer);
  if (!held) return [];
  return Array.from({ length: held.rows }, (_, row) => {
    const at = row * held.columns, id = rowName(held, row) ?? String(row);
    const tag = textCell(held, at + APM.tag);
    const name = held.columns > APM.name ? textCell(held, at + APM.name) : null;
    const nameMissing = held.columns > APM.nameMissing ? textCell(held, at + APM.nameMissing) : null;
    return { id, name, nameMissing, tag, hour: Number(whole(held, at + APM.hour) ?? 0n), summary: valueAt(held, row), points: [] };
  });
}

export function apmHeadStructure(answer: FollowedAnswer | undefined): string {
  if (answer?.kind === 'notRecorded') return `not recorded: ${answer.reason}`;
  const held = rowsOf(answer);
  if (!held) return '';
  let out = '';
  for (let row = 0; row < held.rows; row++) out += `${rowName(held, row)}:${textCell(held, row * held.columns + APM.tag) ?? ''}\u0000`;
  return out;
}

export const apmSortValue = (value: ApmValue): bigint | null =>
  value.apm === null ? null : BigInt(Math.round(value.apm * 1000)) * 2n + (value.gaps ? 1n : 0n);

export function apmFromSort(value: bigint): { apm: number; gaps: boolean } {
  return { apm: Number(value >> 1n) / 1000, gaps: (value & 1n) === 1n };
}

function apmReason(value: ApmValue): string {
  if (!value.complete) return 'Player orders or unpaused time were not fully recorded for this period; the rate is unknown.';
  if (value.unpausedMinutes === 0) return 'No unpaused time was observed in this period.';
  return 'It is unclear whether some orders count toward eAPM; the rate is unknown.';
}

export function apmLiveValues(answer: FollowedAnswer | undefined): LiveValues<SortValue> {
  const values = new Map<string, SortValue>(), reasons = new Map<string, string>();
  for (const row of apmHeadRows(answer)) {
    const put = (column: string, value: SortValue) => { values.set(liveKey(row.id, column), value); if (value == null) reasons.set(row.id, apmReason(row.summary)); };
    put('apm', apmSortValue(row.summary));
    put('orders', row.summary.orders === null ? null : BigInt(row.summary.orders));
    put('paused', row.summary.pausedOrders === null ? null : BigInt(row.summary.pausedOrders));
    put('minutes', row.summary.unpausedMinutes);
  }
  const missing = (key: string) => {
    if (!answer) return 'Not read yet';
    if (answer.kind === 'notRecorded') return answer.reason;
    return reasons.get(key.slice(0, key.lastIndexOf('\u0000'))) ?? 'Not in the period';
  };
  return { get: key => values.get(key) ?? undefined, missing };
}

export function clockHead(answer: FollowedAnswer | undefined): { recorded: number | null; playing: number | null; paused: number | null; complete: boolean } | { reason: string } | null {
  if (!answer) return null;
  if (answer.kind === 'notRecorded') return { reason: answer.reason };
  if (!answer.rows) return { reason: 'Not recorded at this hour' };
  return { recorded: float(answer, CLOCK.recorded), playing: float(answer, CLOCK.playing), paused: float(answer, CLOCK.paused),
    complete: whole(answer, CLOCK.complete) === 1n };
}

export function apmPausedView(head: number | null, first: number, last: number, windowHours: number): [number, number] | null {
  if (head === null) return null;
  const end = Math.max(first, Math.min(last, Math.floor(head)));
  return [Math.max(first, end - windowHours), Math.max(end, first + 1)];
}

export function apmHolders(rows: readonly Pick<ApmSeries, 'id' | 'tag' | 'hour'>[], shown: number | null): { key: string; tag: string; hour: number }[] {
  return rows.flatMap(row => row.tag ? [{ key: row.id, tag: row.tag, hour: shown === null ? row.hour : Math.min(row.hour, shown) }] : []);
}

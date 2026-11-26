import { CELL, cellState, cellValue, rowName, textAt } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer, FollowedRows } from '../../dashboard/follow/message';
import type { ApmGroup, ApmMembership, ApmSeries, ApmValue } from '../../generated/dashboard';

const APM = {
  apm: 0,
  confirmedApm: 1,
  orders: 2,
  pausedOrders: 3,
  minutes: 4,
  complete: 5,
  gaps: 6,
  tag: 7,
  hour: 8,
  name: 9,
  nameMissing: 10,
  confirmedOrders: 11,
  minutesNum: 12,
  minutesDen: 13,
} as const;

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

const rowsOf = (answer: FollowedAnswer | undefined): FollowedRows | null => answer && answer.kind !== 'notRecorded' ? answer : null;

function valueAt(answer: FollowedRows, row: number): ApmValue {
  const at = row * answer.columns;
  const count = (column: number) => whole(answer, at + column)?.toString() ?? null;
  const optText = (column: number) => (answer.columns > column ? textCell(answer, at + column) : null);
  return {
    apm: float(answer, at + APM.apm),
    confirmedApm: float(answer, at + APM.confirmedApm),
    orders: count(APM.orders),
    confirmedOrders: optText(APM.confirmedOrders),
    pausedOrders: count(APM.pausedOrders),
    unpausedMinutes: float(answer, at + APM.minutes),
    unpausedMinutesNum: optText(APM.minutesNum),
    unpausedMinutesDen: optText(APM.minutesDen),
    complete: whole(answer, at + APM.complete) === 1n,
    gaps: whole(answer, at + APM.gaps) === 1n,
  };
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

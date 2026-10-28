import { usNumber } from '../../charts/numberFormat.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatFixed, formatNativeFixed5 } from '../../charts/valueAxis.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { TrainingDivision, TrainingOrder, TrainingQueueReply } from '../../generated/dashboard';

export type QueueRow =
  | { kind: 'order'; key: string; order: TrainingOrder; children: QueueRow[] }
  | { kind: 'division'; key: string; division: TrainingDivision };

const isArray = Array.isArray;
export function validateTrainingQueue(value: unknown): TrainingQueueReply {
  const reply = value as TrainingQueueReply;
  if (!reply || typeof reply.branchId !== 'string' || !Number.isInteger(reply.hour) || typeof reply.tag !== 'string'
    || typeof reply.absent !== 'boolean' || !isArray(reply.orders) || !isArray(reply.warnings)
    || (reply.readHour !== null && !Number.isInteger(reply.readHour))
    || reply.orders.some(order => typeof order.order !== 'string' || !Number.isInteger(order.amount) || !isArray(order.divisions)
      || order.divisions.some(division => typeof division.trainingRaw !== 'string' || !isArray(division.need) || !isArray(division.equipment))))
    throw new Error('Invalid training queue reply');
  return reply;
}

/** Orders in the game's queue order, each with its divisions in training in their order. */
export function queueRows(reply: TrainingQueueReply): QueueRow[] {
  return reply.orders.map(order => ({ kind: 'order', key: `order:${order.order}`, order,
    children: order.divisions.map(division => ({ kind: 'division', key: `line:${division.line}`, division })) }));
}

/** Training as a share of its maximum; null when the maximum is not positive. */
export function trainingRatio(division: TrainingDivision): number | null {
  const maximum = BigInt(division.maxTrainingRaw);
  return maximum > 0n ? fixedNumber(division.trainingRaw, 1) / fixedNumber(division.maxTrainingRaw, 1) : null;
}

/** The share of the template's equipment need a division holds, counting no type past
 * its need; null when the need is unknown or empty, or when a held design's type is
 * unresolved, since what it fills is unknown. */
export function equipmentFill(division: TrainingDivision): number | null {
  if (division.comparisonMissing !== null || !division.need.length) return null;
  let held = 0n, needed = 0n;
  for (const need of division.need) {
    const amount = BigInt(need.needRaw);
    if (amount <= 0n) continue;
    const have = BigInt(need.heldRaw);
    needed += amount;
    held += have < 0n ? 0n : have > amount ? amount : have;
  }
  return needed > 0n ? Number(held * 1_000_000n / needed) / 1_000_000 : null;
}

/** The lowest equipment fill among an order's divisions; null when none is known, or when
 * any division's fill is unknown, since that one could be lower. */
export function lowestFill(order: TrainingOrder): number | null {
  let lowest: number | null = null;
  for (const division of order.divisions) {
    if (division.comparisonMissing !== null) return null;
    const fill = equipmentFill(division);
    if (fill !== null && (lowest === null || fill < lowest)) lowest = fill;
  }
  return lowest;
}

export const orderName = (order: TrainingOrder) => order.templateName ?? 'Template name unavailable';
export const divisionName = (division: TrainingDivision) => division.name ?? 'Division name unavailable';
export const percent = (ratio: number) => `${Math.round(ratio * 100)}%`;

export function orderTooltip(order: TrainingOrder, readHour: number): TooltipContent {
  const rows: TooltipRow[] = [
    { label: 'Ordered', value: usNumber.format(order.amount) },
    { label: 'Divisions in training', value: usNumber.format(order.divisions.length) },
  ];
  const lowest = lowestFill(order);
  if (lowest !== null) rows.push({ label: 'Lowest equipment fill', value: percent(lowest) });
  else if (order.divisions.some(division => division.comparisonMissing !== null)) rows.push({ label: 'Lowest equipment fill', value: 'Not recorded', tone: 'muted' });
  rows.push({ label: 'Location', value: order.locationName ?? 'Not recorded' }, { label: 'Priority', value: order.priorityName ?? 'Not recorded' });
  if (order.closed) rows.push({ label: 'Closed', value: 'Yes' });
  if (order.exile) rows.push({ label: 'Government in exile', value: order.exile });
  return { title: orderName(order), subtitle: `As of ${formatDateTime(readHour)}`, rows };
}

export function divisionTooltip(division: TrainingDivision, readHour: number): TooltipContent {
  const training = trainingRatio(division);
  const rows: TooltipRow[] = [
    { label: 'Training', value: `${formatNativeFixed5(division.trainingRaw, 2)} of ${formatNativeFixed5(division.maxTrainingRaw, 2)}${training === null ? '' : ` (${percent(training)})`}` },
    { label: 'Manpower available', value: usNumber.format(division.manpower) },
    { label: 'Manpower needed', value: usNumber.format(division.manpowerNeeded) },
  ];
  const contributors = division.contributors.filter(entry => !entry.needed);
  if (contributors.length > 1) for (const entry of contributors)
    rows.push({ label: entry.tag ?? 'Country not recorded', value: usNumber.format(entry.amount), indent: true });
  if (division.needMissing !== null) rows.push({ label: 'Equipment', value: 'Not recorded', divider: true });
  else if (division.need.length) {
    rows.push({ label: 'Equipment', strong: true, divider: true });
    for (const need of division.need)
      rows.push({ label: need.name ?? 'Equipment name unavailable', lookupKey: need.archetype,
        value: `${formatFixed(need.heldRaw, 5, 0)} / ${formatFixed(need.needRaw, 5, 0)}`, indent: true });
    if (division.comparisonMissing !== null) rows.push({ label: 'Fill not recorded', value: division.comparisonMissing, tone: 'muted', indent: true });
  }
  if (division.halted) rows.push({ label: 'Halted', value: 'Halting reason not available', divider: true });
  return { title: divisionName(division), subtitle: `As of ${formatDateTime(readHour)}`, rows };
}

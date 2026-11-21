import { formatAmount } from '../../charts/valueAxis.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { FuelBranchesReply } from '../../generated/dashboard';

export const FUEL_BRANCHES = 'fuel-branches';
export const FUEL_CONSUMERS = 'fuel-consumers';
export const FUEL_PLACES = 5;
const MOST_FUEL_DECIMALS = 5;

export const FUEL_DECIMALS_OPTION: OptionField = { key: 'fuelDecimals', section: 'options', label: 'Fuel decimals',
  hint: 'Decimal places in fuel amounts.', kind: 'number', min: 0, max: MOST_FUEL_DECIMALS, step: 1, default: 0 };

export function fuelDecimals(options: Readonly<Record<string, OptionValue>>): number {
  const chosen = options[FUEL_DECIMALS_OPTION.key];
  return typeof chosen === 'number' && Number.isInteger(chosen) ? Math.max(0, Math.min(MOST_FUEL_DECIMALS, chosen)) : 0;
}

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);

export function validateFuelBranches(value: unknown): FuelBranchesReply {
  if (!object(value) || typeof value.tag !== 'string' || value.scale !== 100000 || !Array.isArray(value.consumerNames)
    || !value.consumerNames.every(name => name === null || typeof name === 'string')
    || !(value.readHour === null || Number.isSafeInteger(value.readHour)) || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')
    || !Array.isArray(value.consumers) || !value.consumers.every((row, at, rows) => object(row) && Number.isSafeInteger(row.index)
      && (at === 0 || (rows[at - 1] as { index: number }).index < (row.index as number)) && raw(row.requested) && raw(row.received)
      && Number.isSafeInteger(row.priority)))
    throw new Error('The fuel by branch reply is malformed.');
  return value as unknown as FuelBranchesReply;
}

export interface BranchAt {
  index: number;
  name: string;
  requested: bigint;
  received: bigint;
  unmet: bigint;
  priority: number;
}

export const branchName = (names: readonly (string | null)[], index: number) => names[index] ?? 'Branch name unavailable';

export function branchesAt(reply: FuelBranchesReply): BranchAt[] {
  return reply.consumers.map(row => {
    const requested = BigInt(row.requested), received = BigInt(row.received);
    return { index: row.index, name: branchName(reply.consumerNames, row.index), requested, received,
      unmet: requested > received ? requested - received : 0n, priority: row.priority };
  });
}

export function fuelStructure(reply: FuelBranchesReply): string {
  return JSON.stringify([reply.missing, reply.consumerNames, reply.consumers.map(row => [row.index, row.requested === '0', BigInt(row.requested) > BigInt(row.received)])]);
}

export function barWidths(rows: readonly BranchAt[]): Map<number, { delivered: string; unmet: string }> {
  let largest = 0n;
  for (const row of rows) for (const value of [row.requested, row.received]) if (value > largest) largest = value;
  const width = (value: bigint) => largest > 0n ? `${Number(value * 100000n / largest) / 1000}%` : '0%';
  return new Map(rows.map(row => [row.index, { delivered: width(row.received < row.requested ? row.received : row.requested), unmet: width(row.unmet) }]));
}

export const fuelAmount = (value: bigint, digits: number) => formatAmount(value, FUEL_PLACES, digits);

export const percentText = (value: number) => `${value.toFixed(value % 1 === 0 ? 0 : 2)}%`;

export function ratioAxis(most: number): { top: number; ticks: number[] } {
  const highest = Math.max(100, most);
  let step = 25;
  for (let scale = 1; Math.ceil(highest / step) > 8; scale *= 10) step = [25, 50, 100].map(base => base * scale).find(size => Math.ceil(highest / size) <= 8) ?? 100 * scale;
  const top = Math.ceil(highest / step) * step;
  return { top, ticks: Array.from({ length: top / step + 1 }, (_, index) => index * step) };
}

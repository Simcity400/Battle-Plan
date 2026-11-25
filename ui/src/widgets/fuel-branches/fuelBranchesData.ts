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
const optText = (value: unknown) => value === null || typeof value === 'string';
const optTextList = (value: unknown) => Array.isArray(value) && value.every(optText);

export function validateFuelBranches(value: unknown): FuelBranchesReply {
  if (!object(value) || typeof value.tag !== 'string' || value.scale !== 100000
    || !optTextList(value.consumerNames)
    || !optTextList(value.consumerIcons)
    || !optTextList(value.consumerIconReasons)
    || !optText(value.fuelIcon)
    || !optText(value.fuelIconReason)
    || !(value.readHour === null || Number.isSafeInteger(value.readHour)) || !optText(value.missing)
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')
    || !Array.isArray(value.consumers) || !value.consumers.every((row, at, rows) => object(row) && Number.isSafeInteger(row.index)
      && (at === 0 || (rows[at - 1] as { index: number }).index < (row.index as number)) && raw(row.requested) && raw(row.received)
      && Number.isSafeInteger(row.priority)))
    throw new Error('The fuel by branch reply is malformed.');
  return value as unknown as FuelBranchesReply;
}

export const branchName = (names: readonly (string | null)[], index: number) => names[index] ?? 'Branch name unavailable';

export const fuelAmount = (value: bigint, digits: number) => formatAmount(value, FUEL_PLACES, digits);

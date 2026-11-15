import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatFixed } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { DivisionEquipmentReply, DivisionPoolType, EquipmentDivision, EquipmentTemplate, EquipmentType, EquipmentVariant } from '../../generated/dashboard';

export type { DivisionEquipmentReply, DivisionPoolType, EquipmentDivision, EquipmentTemplate, EquipmentType };

/** A type's key: its archetype, or the unclassified bucket. Tagged, so a mod archetype
 * named like the bucket is still its own type. */
export const typeKey = (row: { archetype: string | null }) => JSON.stringify([row.archetype]);

const isArray = Array.isArray;
export function validateDivisionEquipment(value: unknown): DivisionEquipmentReply {
  const reply = value as DivisionEquipmentReply;
  if (!reply || typeof reply.tag !== 'string' || (reply.missing !== null && typeof reply.missing !== 'string')
    || !isArray(reply.types) || !isArray(reply.templates) || !isArray(reply.warnings)
    || reply.types.some(row => (row.archetype !== null && typeof row.archetype !== 'string') || (row.heldRaw !== null && typeof row.heldRaw !== 'string') || !isArray(row.variants)))
    throw new Error('Invalid division equipment reply');
  return reply;
}

/** What a followed reply renders on: everything it shows (its numbers move at most daily, but for a template's
 * manpower and strength). */
export const equipmentStructure = (reply: DivisionEquipmentReply) => JSON.stringify(reply);

/** A count, or that it is not recorded. */
export const counted = (value: number | null) => value === null ? 'Not recorded' : usNumber.format(value);

/** Whole items from a fixed-point amount, grouped. */
export const items = (raw: string | null) => raw === null ? null : formatFixed(raw, 5, 0);
const big = (raw: string | null) => raw === null ? null : BigInt(raw);

/** Held as a share of need, never above one; null when the need is unknown or none. */
export function fill(held: string | null, needed: string | null): number | null {
  const have = big(held), need = big(needed);
  if (have === null || need === null || need <= 0n) return null;
  const capped = have < 0n ? 0n : have > need ? need : have;
  return Number(capped * 1_000_000n / need) / 1_000_000;
}

/** A type's fill: each known division's holding capped at its own need, summed, as a
 * share of the summed need, so one division's surplus never fills another's shortfall. */
export const typeFill = (row: EquipmentType) => fill(row.filledRaw, row.neededRaw);

/** A current/maximum pair as a share; null when either is unknown or the maximum is not positive. */
export function share(current: string | null, maximum: string | null): number | null {
  const value = big(current), top = big(maximum);
  if (value === null || top === null || top <= 0n) return null;
  return Math.max(0, Number(value * 1_000_000n / top) / 1_000_000);
}

export const typeName = (row: { name: string | null; archetype?: string | null }) => row.name ?? (row.archetype === null ? 'Unclassified equipment' : 'Equipment name unavailable');
export const templateName = (row: { name: string | null }) => row.name ?? 'Template name unavailable';
export const divisionName = (row: { name: string | null }) => row.name ?? 'Division name unavailable';
export const percent = (ratio: number) => `${Math.round(ratio * 100)}%`;

function variantRows(variants: readonly EquipmentVariant[]): TooltipRow[] {
  return variants.map(variant => ({ label: variant.name ?? 'Design name unavailable', lookupKey: variant.variant, value: items(variant.heldRaw) ?? '—', indent: true }));
}

export function typeTooltip(row: EquipmentType, reply: DivisionEquipmentReply): TooltipContent {
  const rows: TooltipRow[] = [
    { label: 'In divisions', value: items(row.heldRaw) ?? 'Not recorded' },
    { label: 'Needed', value: items(row.neededRaw) ?? 'Not recorded' },
    { label: 'Filled', value: items(row.filledRaw) ?? 'Not recorded' },
    { label: 'Missing', value: items(row.missingRaw) ?? 'Not recorded', tone: row.missingRaw !== null && BigInt(row.missingRaw) > 0n ? 'negative' : undefined },
    { label: 'Divisions short', value: counted(row.divisionsShort) },
  ];
  if (row.unknownDivisions === null || row.unknownDivisions > 0) rows.push({ label: 'Divisions not recorded', value: counted(row.unknownDivisions), tone: 'muted' });
  if (row.variants.length) rows.push({ label: 'Models', strong: true, divider: true }, ...variantRows(row.variants));
  if (row.stockpileRaw !== null) rows.push({ label: 'In stockpile', value: `${items(row.stockpileRaw)}${row.stockpileHour === null ? '' : ` (as of ${formatDateTime(row.stockpileHour)})`}`, divider: true });
  return { title: typeName(row), subtitle: reply.poolHour === null ? undefined : `Equipment as of ${formatDateTime(reply.poolHour)}`, rows };
}

export function templateTooltip(row: EquipmentTemplate, reply: DivisionEquipmentReply): TooltipContent {
  const rows: TooltipRow[] = [
    { label: 'Divisions', value: counted(row.divisions) },
    { label: 'Manpower', value: row.manpower === null ? 'Not recorded' : `${usNumber.format(Number(row.manpower))}${row.manpowerRequired === null ? '' : ` of ${usNumber.format(Number(row.manpowerRequired))}`}` },
    { label: 'Strength', value: row.hpRaw === null ? 'Not recorded' : `${formatFixed(row.hpRaw, 5, 1)}${row.hpMaxRaw === null ? '' : ` of ${formatFixed(row.hpMaxRaw, 5, 1)}`}` },
    { label: 'Equipment', value: row.neededRaw === null ? 'Not recorded' : `${items(row.heldRaw) ?? '—'} of ${items(row.neededRaw)}` },
    { label: 'Types short', value: counted(row.typesShort) },
  ];
  if (row.unknownDivisions === null || row.unknownDivisions > 0) rows.push({ label: 'Divisions not recorded', value: counted(row.unknownDivisions), tone: 'muted' });
  return { title: templateName(row), subtitle: reply.poolHour === null ? undefined : `Equipment as of ${formatDateTime(reply.poolHour)}`, rows };
}

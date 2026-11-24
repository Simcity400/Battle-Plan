import { contentTitle } from '../../dashboard/contentTitles.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { object } from '../../dashboard/validation.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import type { EquipmentIcon, LineNow, OrganisationName, OutputNow, ProductionLineBreakdownReply, ProductionLinesAtReply, VariantName } from '../../generated/dashboard';

const PART = /^-?\d+$/;
const ID = /^\d{1,20}$/;
const HOUR = (value: unknown): value is number => Number.isInteger(value);
const KINDS = ['military', 'naval', 'shipRefit', 'railwayGun'];
const TEXT = (value: unknown) => value === null || typeof value === 'string';
const ordered = <T extends { firstHour: number; lastHour: number }>(runs: readonly T[]) =>
  runs.every((run, index) => HOUR(run.firstHour) && HOUR(run.lastHour) && run.firstHour <= run.lastHour && (index === 0 || runs[index - 1].lastHour < run.firstHour));
const isVariant = (variant: VariantName) => object(variant) && ID.test(variant.id) && (variant.hour === undefined || HOUR(variant.hour)) && typeof variant.known === 'boolean'
  && typeof variant.token === 'string' && [variant.name, variant.typeName, variant.nameUnknown, variant.icon, variant.iconUnknown].every(TEXT);
const isOrganisation = (organisation: OrganisationName) => object(organisation) && ID.test(organisation.id) && (organisation.hour === undefined || HOUR(organisation.hour))
  && typeof organisation.token === 'string' && [organisation.name, organisation.nameUnknown, organisation.icon, organisation.iconUnknown].every(TEXT);
const isIcon = (icon: EquipmentIcon) => object(icon) && typeof icon.key === 'string' && typeof icon.image === 'string';

function invalid(what: string): never {
  throw new Error(`Invalid ${what} reply`);
}

const isOutputNow = (output: OutputNow | null) => output === null || object(output) && Array.isArray(output.parts)
  && output.parts.every(part => object(part) && ID.test(part.variant) && ID.test(part.mio) && typeof part.exact === 'boolean') && HOUR(output.sinceHour) && typeof output.wholeLife === 'boolean';
const isNow = (line: LineNow) => object(line) && ID.test(line.handle) && (line.kind === null || KINDS.includes(line.kind)) && ID.test(line.variant) && ID.test(line.mio)
  && [line.maxFactories, line.priority].every(Number.isInteger) && isOutputNow(line.output);

export function validateAtReply(data: unknown): ProductionLinesAtReply {
  const reply = data as ProductionLinesAtReply;
  if (!object(reply) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !Array.isArray(reply.lines) || !reply.lines.every(isNow)
    || !TEXT(reply.missing) || !TEXT(reply.outputMissing) || !Array.isArray(reply.variants) || !reply.variants.every(isVariant)
    || !Array.isArray(reply.organisations) || !reply.organisations.every(isOrganisation) || !Array.isArray(reply.icons) || !reply.icons.every(isIcon)
    || !Array.isArray(reply.warnings)) invalid('production lines at an hour');
  return reply;
}

export function validateBreakdownReply(data: unknown): ProductionLineBreakdownReply {
  const reply = data as ProductionLineBreakdownReply;
  if (!object(reply) || typeof reply.key !== 'string' || !HOUR(reply.firstHour) || !HOUR(reply.lastHour) || !Array.isArray(reply.groups)
    || !Array.isArray(reply.runs) || !ordered(reply.runs) || !reply.runs.every(run => Array.isArray(run.groups) && run.groups.every(group =>
      typeof group.id === 'string' && TEXT(group.totalRaw) && Array.isArray(group.parts) && group.parts.every(part => typeof part.id === 'string' && PART.test(part.raw))))
    || !Array.isArray(reply.warnings)) invalid('production line breakdown');
  return reply;
}

export function breakdownAt(reply: ProductionLineBreakdownReply, hour: number, group: string): ProductionLineBreakdownReply['runs'][number]['groups'][number] | null {
  const run = reply.runs.find(item => item.firstHour <= hour && hour <= item.lastHour);
  return run?.groups.find(item => item.id === group) ?? null;
}

export const variantOf = (variants: readonly VariantName[], id: string): VariantName | null => variants.find(variant => variant.id === id && variant.known) ?? null;
export const organisationOf = (organisations: readonly OrganisationName[], id: string): OrganisationName | null =>
  id === '0' ? null : organisations.find(organisation => organisation.id === id) ?? null;

export const variantName = (variant: VariantName | null, id: string) => variant ? variant.name ?? variant.typeName ?? 'Variant name unavailable' : `Variant ${id}`;

export const organisationText = (organisation: OrganisationName | null, mio: string) =>
  mio === '0' ? '—' : organisation ? contentTitle(organisation.name, organisation.token, 'Organisation name unavailable') : 'Not recorded';

export const iconImage = (icons: readonly EquipmentIcon[], key: string | null): string | null =>
  key == null ? null : drawableIcon(icons.find(icon => icon.key === key)?.image);

export function weeklyText(weekly: bigint, decimals: number): string {
  const short = formatAmount(weekly, decimals, 1);
  return short === '0' && weekly !== 0n ? formatAmount(weekly, decimals, decimals) : short;
}

const EFFICIENCY_FULL = 3276800n;
export const efficiencyPercent = (raw: bigint) => (raw + 0x4000n) >> 15n;
export const efficiencyFraction = (raw: bigint) => Math.max(0, Math.min(1, Number(raw) / Number(EFFICIENCY_FULL)));

export function weeklyRaw(speed: bigint | null, cost: bigint | null): bigint | null {
  return speed === null || cost === null || cost <= 0n ? null : speed * 700_000n / cost;
}

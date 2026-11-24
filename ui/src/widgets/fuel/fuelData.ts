import type { ChartSummary } from '../../charts/chartSummary';
import type { Run } from '../../charts/series';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatSigned, rescale } from '../../charts/valueAxis.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { object } from '../../dashboard/validation.ts';
import type { CountryFuelReply, DayLeftOut, FuelAwardHistoryReply, FuelAwardsReply, FuelLabel, FuelReadAt } from '../../generated/dashboard';

const MOST_FUEL_DECIMALS = 5;
export const FUEL_DECIMALS_OPTION: OptionField = { key: 'fuelDecimals', section: 'options', label: 'Fuel decimals',
  hint: 'Decimal places in fuel amounts.', kind: 'number', min: 0, max: MOST_FUEL_DECIMALS, step: 1, default: 0 };

export function fuelDecimals(options: Readonly<Record<string, OptionValue>>): number {
  const chosen = options[FUEL_DECIMALS_OPTION.key];
  return typeof chosen === 'number' && Number.isInteger(chosen) ? Math.max(0, Math.min(MOST_FUEL_DECIMALS, chosen)) : 0;
}

export const COUNTRY_FUEL = 'country-fuel';
export const FUEL_DAYS = 'fuel-days';
export const FUEL_DEMAND_TOTALS = 'fuel-demand-totals';
export const QUANTITY_SCALE = 32768;
export const GAIN_SCALE = 100000;
export const PART_SCALE = QUANTITY_SCALE * GAIN_SCALE;

export const GAIN_PARTS = [['gainBase', 'gain-0'], ['gainStates', 'gain-1'], ['gainOil', 'gain-2'], ['gainNegativeCost', 'gain-3'],
  ['unidentified', 'unidentified']] as const;
export const BURN_PARTS = [['consumerArmy', 'consumer-0'], ['consumerAir', 'consumer-1'], ['consumerNavy', 'consumer-2'],
  ['outgoing', 'outgoing'], ['flat', 'flat'], ['other', 'other'], ['conversion', 'conversion']] as const;
export const FAMILY_MEASURES = ['positiveArmy', 'positiveAir', 'positiveNavy'] as const;

const RAW = /^-?\d{1,39}$/;
export const isRaw = (value: unknown): value is string => typeof value === 'string' && RAW.test(value);
const rawOrNull = (value: unknown) => value === null || isRaw(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const int = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
const hourOrNull = (value: unknown) => value === null || int(value);
const triple = (value: unknown, each: (item: unknown) => boolean) => Array.isArray(value) && value.length === 3 && value.every(each);
const strings = (value: unknown) => Array.isArray(value) && value.every(item => typeof item === 'string');
const isLabel = (value: unknown): value is FuelLabel => object(value) && typeof value.key === 'string' && text(value.template)
  && Array.isArray(value.arguments) && value.arguments.every(arg => arg === 'NUM' || arg === 'OIL') && text(value.reason);
const LEFT_OUT_KINDS: readonly DayLeftOut[] = ['overlaps', 'notUpdated', 'ownershipNotRecorded', 'notRecorded'];

function isRead(value: unknown): value is FuelReadAt {
  if (!object(value)) return false;
  const day = value.day;
  const awards = object(day) ? day.awards : null;
  return hourOrNull(value.readHour) && int(value.country) && (value.countryInstance === null || int(value.countryInstance))
    && isRaw(value.fuelRaw) && isRaw(value.capacityRaw) && isRaw(value.lendLeaseRaw) && hourOrNull(value.dayHour)
    && text(value.awardsReason) && hourOrNull(value.continuityStartHour)
    && (day === null ? typeof value.leftOut === 'string' && LEFT_OUT_KINDS.includes(value.leftOut as DayLeftOut)
      : value.leftOut === null && object(day) && isRaw(day.gainedRaw) && isRaw(day.burnedRaw) && isRaw(day.otherRaw) && triple(day.receivedRaw, isRaw)
        && (awards === null || object(awards) && Array.isArray(awards.gainsRaw) && awards.gainsRaw.length === 4 && awards.gainsRaw.every(isRaw)
          && isRaw(awards.ownGainRaw) && isRaw(awards.ownGainQuantityRaw) && Array.isArray(awards.transfersRaw) && awards.transfersRaw.length === 4
          && awards.transfersRaw.every(isRaw) && isRaw(awards.consumedRaw) && triple(awards.receivedRaw, isRaw)));
}

export function validateCountryFuel(data: unknown): CountryFuelReply {
  const reply = data as CountryFuelReply;
  const consumption = object(data) ? data.consumption : null;
  const ok = object(data) && typeof reply.tag === 'string' && int(reply.quantityScale) && reply.quantityScale > 0 && int(reply.gainScale) && reply.gainScale > 0
    && int(reply.dayHours) && reply.dayHours > 0 && int(reply.detailHours) && reply.detailHours > 0 && triple(reply.consumerNames, text)
    && Array.isArray(reply.gainLabels) && reply.gainLabels.length === 4 && reply.gainLabels.every(isLabel)
    && Array.isArray(reply.transferLabels) && reply.transferLabels.length === 3 && reply.transferLabels.every(isLabel)
    && (reply.read === null ? typeof reply.missing === 'string' : isRead(reply.read) && reply.missing === null)
    && (consumption === null ? typeof reply.consumptionMissing === 'string' : object(consumption) && triple(consumption.nativeDailyRaw, rawOrNull)
      && triple(consumption.knownDailyRaw, rawOrNull) && triple(consumption.positiveDailyRaw, rawOrNull) && triple(consumption.reasons, text))
    && strings(reply.warnings);
  if (!ok) throw new Error('The fuel reply is malformed.');
  return reply;
}

export function validateFuelAwards(data: unknown): FuelAwardsReply {
  const reply = data as FuelAwardsReply;
  if (!object(data) || typeof reply.tag !== 'string' || !int(reply.readHour) || !int(reply.country) || !int(reply.countryInstance)
    || !triple(reply.receivedRaw, isRaw) || !Array.isArray(reply.recipients) || !strings(reply.warnings)
    || !reply.recipients.every(part => object(part) && typeof part.key === 'string' && part.key.length > 0 && int(part.kind) && part.kind >= 0 && part.kind <= 2
      && typeof part.reference === 'string' && int(part.owner) && typeof part.host === 'string' && int(part.firstHour) && int(part.lastHour)
      && isRaw(part.amountRaw) && text(part.recordedName) && text(part.nameReason))
    || new Set(reply.recipients.map(part => part.key)).size !== reply.recipients.length) throw new Error('The fuel recipient reply is malformed.');
  return reply;
}

export function validateFuelAwardHistory(data: unknown): FuelAwardHistoryReply {
  const reply = data as FuelAwardHistoryReply;
  if (!object(data) || typeof reply.tag !== 'string' || !int(reply.country) || !int(reply.window) || typeof reply.recipientKey !== 'string'
    || !int(reply.firstHour) || !int(reply.lastHour) || !Array.isArray(reply.runs) || !strings(reply.warnings)
    || !reply.runs.every((run, i) => object(run) && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour && typeof run.startsSegment === 'boolean'
      && (run.amountRaw === null ? typeof run.reason === 'string' : isRaw(run.amountRaw) && run.reason === null)
      && (i === 0 || reply.runs[i - 1].lastHour < run.firstHour))) throw new Error('The fuel recipient history is malformed.');
  return reply;
}

export const LEFT_OUT: Readonly<Record<DayLeftOut, { value: string; note: string }>> = {
  overlaps: { value: 'Counted in another day', note: 'Included in another day.' },
  notUpdated: { value: 'Not updated', note: 'Fuel was not updated while no states were owned.' },
  ownershipNotRecorded: { value: 'Not known', note: 'Daily data incomplete.' },
  notRecorded: { value: 'Not recorded', note: 'The day before this read was not recorded.' },
};

export const formatFuel = (raw: string | bigint, scale: number, digits: number) => formatAmount(rescale(BigInt(raw), BigInt(scale), digits), digits, digits);

export function formatFuelPart(raw: bigint, scale: bigint, digits?: number): string {
  if (digits !== undefined) return formatAmount(rescale(raw, scale, digits), digits, digits);
  let denominator = scale, twos = 0, fives = 0;
  while (denominator % 2n === 0n) { denominator /= 2n; twos++; }
  while (denominator % 5n === 0n) { denominator /= 5n; fives++; }
  if (denominator !== 1n) return `${raw}/${scale}`;
  const places = Math.max(twos, fives);
  return formatAmount(rescale(raw, scale, places), places, places);
}

export const fuelPartShare = (raw: bigint, total: bigint) => total === 0n ? '—'
  : `${formatAmount(rescale(raw * (total < 0n ? -100n : 100n), total < 0n ? -total : total, 1), 1, 1)}%`;

export function dayAmounts(reply: Pick<CountryFuelReply, 'quantityScale' | 'gainScale'>, read: Pick<FuelReadAt, 'day'>, digits: number) {
  if (!read.day) return null;
  const [gainScale, quantityScale] = [BigInt(reply.gainScale), BigInt(reply.quantityScale)];
  const gained = BigInt(read.day.gainedRaw), burned = BigInt(read.day.burnedRaw);
  const [net, netScale] = read.day.awards ? [BigInt(read.day.awards.ownGainQuantityRaw) - burned, quantityScale]
    : [gained * quantityScale - burned * gainScale, gainScale * quantityScale];
  return { gained: rescale(gained, gainScale, digits), burned: rescale(burned, quantityScale, digits), net: rescale(net, netScale, digits), gaining: net > 0n };
}

export function percentFull(read: Pick<FuelReadAt, 'fuelRaw' | 'capacityRaw'>): bigint | null {
  const capacity = BigInt(read.capacityRaw);
  return capacity > 0n ? rescale(BigInt(read.fuelRaw) * 100n, capacity, 0) : null;
}

export interface FuelPart { id: string; label: string; raw: bigint; consumer?: number; muted?: boolean; reason?: string | null }

export function fuelLabel(label: FuelLabel | undefined, raw: bigint, scale: bigint, digits?: number): string | null {
  return label?.template?.replaceAll('{NUM}', formatFuelPart(raw, scale, digits)).replaceAll('{OIL}', '?') ?? null;
}

export function fuelParts(reply: CountryFuelReply, read: FuelReadAt, measure: 'gain' | 'burn', digits?: number):
  { scale: bigint; total: bigint; parts: FuelPart[]; transfer: FuelPart | null; reason: string | null } | null {
  if (!read.day) return null;
  const gain = BigInt(reply.gainScale), quantity = BigInt(reply.quantityScale), awards = read.day.awards;
  const named = (id: string, raw: bigint, label: string | null | undefined, reason?: string | null): FuelPart => ({ id, raw, label: label ?? 'Other', muted: label == null, reason });
  if (measure === 'gain') return { scale: gain, total: BigInt(read.day.gainedRaw),
    parts: awards ? awards.gainsRaw.map((raw, i) => named(`gain-${i}`, BigInt(raw), fuelLabel(reply.gainLabels[i], BigInt(raw), gain, digits), reply.gainLabels[i].reason))
      : [{ id: 'unidentified', label: 'Other', raw: BigInt(read.day.gainedRaw), muted: true }],
    transfer: awards ? named('incoming', BigInt(awards.transfersRaw[0]), fuelLabel(reply.transferLabels[0], BigInt(awards.transfersRaw[0]), quantity, digits), reply.transferLabels[0].reason) : null,
    reason: awards ? null : read.awardsReason };
  const total = BigInt(read.day.burnedRaw) * gain;
  const received = read.day.receivedRaw.map((raw, index) => ({ ...named(`consumer-${index}`, BigInt(raw) * quantity, reply.consumerNames[index]), consumer: index }));
  const other = BigInt(read.day.otherRaw) * gain;
  const residual = total - other - received.reduce((sum, part) => sum + part.raw, 0n);
  const transfers: FuelPart[] = awards ? [
    named('outgoing', BigInt(awards.transfersRaw[1]) * gain, fuelLabel(reply.transferLabels[1], BigInt(awards.transfersRaw[1]), quantity, digits), reply.transferLabels[1].reason),
    named('flat', BigInt(awards.transfersRaw[2]) * gain, fuelLabel(reply.transferLabels[2], BigInt(awards.transfersRaw[2]), quantity, digits), reply.transferLabels[2].reason),
    { id: 'other', label: 'Other', raw: other - (BigInt(awards.transfersRaw[1]) + BigInt(awards.transfersRaw[2])) * gain, muted: true }]
    : [{ id: 'other', label: 'Other', raw: other, muted: true }];
  return { scale: gain * quantity, total, parts: [...received, ...transfers, { id: 'conversion', label: 'Hourly conversion difference', raw: residual }],
    transfer: null, reason: awards ? null : read.awardsReason };
}

export function partsStructure(reply: CountryFuelReply | undefined, measure: 'gain' | 'burn'): string {
  const read = reply?.read;
  const values = reply && read ? fuelParts(reply, read, measure) : null;
  return JSON.stringify([reply?.missing ?? null, read?.leftOut ?? null, read?.day?.awards ? 1 : 0, values?.reason ?? null,
    values?.parts.map(part => [part.id, part.label, part.reason ?? null]) ?? null, values?.transfer?.label ?? null, reply?.consumerNames ?? null]);
}

export function fuelTooltips(reply: CountryFuelReply, title: string,
  colors: { fuel: string; capacity: string; gained: string; burned: string }, digits: number): TooltipContent[] {
  const read = reply.read;
  if (!read) return [{ title, rows: [{ label: reply.missing ?? 'Not recorded at this time', tone: 'muted' }] }];
  const full = percentFull(read);
  const tank: TooltipRow[] = [
    { label: 'Fuel', value: formatFuel(read.fuelRaw, reply.quantityScale, digits), swatch: colors.fuel },
    { label: 'Capacity', value: formatFuel(read.capacityRaw, reply.quantityScale, digits), swatch: colors.capacity, tone: 'muted' },
    { label: 'Full', value: full == null ? 'No capacity' : `${full}%` },
  ];
  const notes: string[] = [];
  const day = dayAmounts(reply, read, digits);
  let rows: TooltipRow[];
  if (day) {
    const net: TooltipRow = { label: 'Net', value: formatSigned(day.net, digits, digits), strong: true,
      tone: day.net < 0n ? 'negative' : day.net > 0n ? 'positive' : undefined };
    rows = [...tank, { label: 'Gained (own production)', value: formatSigned(day.gained, digits, digits), swatch: colors.gained, divider: true },
      { label: 'Burned', value: formatSigned(-day.burned, digits, digits), swatch: colors.burned }, net];
    notes.push(`Previous ${reply.dayHours} hours; excludes incoming lend-lease.`);
    if (read.fuelRaw === read.capacityRaw && day.gaining) notes.push('The tank was full: the game discards fuel beyond its capacity.');
  } else {
    const why = LEFT_OUT[read.leftOut ?? 'notRecorded'];
    rows = [...tank, { label: 'Gained and burned', value: why.value, tone: 'muted', divider: true }];
    notes.push(why.note);
  }
  const incoming = read.day?.awards?.transfersRaw[0];
  if (day && incoming !== undefined) rows.push({ label: 'Lend-lease received (separate)', value: formatSigned(rescale(BigInt(incoming), BigInt(reply.quantityScale), digits), digits, digits) });
  else if (day && BigInt(read.lendLeaseRaw) !== 0n) notes.push('Daily lend-lease total unavailable.');
  const note = notes.join(' ');
  return [{ title, rows, note }, { title, rows: [tank[0], rows.find(row => row.strong) ?? rows.at(-1)!], note }];
}

export function scaledSummary(summary: ChartSummary | undefined, scale: number): ChartSummary | undefined {
  return summary && { ...summary, tiles: undefined, runs: summary.runs?.map(run => ({ ...run, value: run.value / scale })),
    buckets: summary.buckets.map(bucket => bucket && ({ ...bucket, firstValue: bucket.firstValue / scale, lastValue: bucket.lastValue / scale,
      minValue: bucket.minValue / scale, maxValue: bucket.maxValue / scale })) };
}

export function summaryRuns(summary: ChartSummary | undefined): Run[] {
  if (!summary) return [];
  if (summary.runs) return [...summary.runs];
  return summary.buckets.flatMap(bucket => {
    if (!bucket) return [];
    const ordered = [...new Map([[bucket.firstHour, bucket.firstValue], [bucket.minHour, bucket.minValue], [bucket.maxHour, bucket.maxValue],
      [bucket.lastValueHour, bucket.lastValue], [bucket.lastHour, bucket.lastValue]])].sort((a, b) => a[0] - b[0]);
    return ordered.map(([hour, value], index) => ({ firstHour: hour, lastHour: bucket.hasBreak ? hour : ordered[index + 1] ? ordered[index + 1][0] - 1 : bucket.lastHour,
      value, breakBefore: bucket.hasBreak || index === 0 && bucket.firstBreak }));
  });
}

import type { DayLeftOut, FuelAwardTotals, FuelAwardsReply, FuelAwardHistoryReply, FuelAwardRun, FuelLabel, FuelRead, FuelReply } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import type { ResolvedCountries } from '../../dashboard/groups';
import { isMember } from '../../dashboard/groups.ts';
import { object, signed32, unsigned32, unsigned64 } from '../../dashboard/validation.ts';
import { readAt, readNote } from '../../dashboard/reads.ts';
import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import type { StripBar } from '../../charts/strip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatSigned, rescale } from '../../charts/valueAxis.ts';

/** Content labels reload independently of recorded amounts and native identity. */
const labelNumbers = (label: FuelLabel): FuelLabel => ({ ...label, template: null, reason: null });
export const fuelNumbers = (replies: FuelReply[]): FuelReply[] => replies.map(reply => ({ ...reply,
  consumerNames: [null, null, null], warnings: [],
  gainLabels: reply.gainLabels.map(labelNumbers) as FuelReply['gainLabels'],
  transferLabels: reply.transferLabels.map(labelNumbers) as FuelReply['transferLabels'] }));
export const fuelAwardNumbers = (reply: FuelAwardsReply): FuelAwardsReply => ({ ...reply, warnings: [],
  awards: { ...reply.awards, recipients: reply.awards.recipients.map(part => ({ ...part, recordedName: null, nameReason: null })) } });

/** The candidate source is branch-wide, so ordinary playback does not change it. */
export function fuelChoiceKey(selection: CountrySelection, countries: Pick<ResolvedCountries, 'tags' | 'membership'>, capacity: boolean): string {
  return JSON.stringify([selection, countries.tags, countries.membership ? [...countries.membership].map(([tag, runs]) =>
    [tag, runs.map(run => [run.firstHour, run.lastHour])]) : null, capacity]);
}

const RAW = /^-?\d{1,39}$/;
const RAW_MAX = (1n << 127n) - 1n;
const RAW_MIN = -RAW_MAX - 1n;
/** Applied daily operands are summed into signed i128, beyond one native i64. */
export function isRaw(value: unknown): value is string {
  if (typeof value !== 'string' || !RAW.test(value)) return false;
  const raw = BigInt(value);
  return raw >= RAW_MIN && raw <= RAW_MAX;
}
/** Why a day is left out, as the tooltip says it: its row's value and its note. */
const LEFT_OUT: Readonly<Record<DayLeftOut, (dayHours: number) => { value: string; note: string }>> = {
  overlaps: dayHours => ({ value: 'Counted in another day',
    note: `This read is less than ${dayHours} hours from another, so its hours are counted in that read’s bar.` }),
  notUpdated: () => ({ value: 'Not updated',
    note: 'The country held no state for part of the day, and the game updates a country’s fuel only while it holds one.' }),
  ownershipNotRecorded: () => ({ value: 'Not known',
    note: 'Part of the day was not recorded, or recorded without the states each country owned, so whether the game updated this fuel is not known.' }),
};
/** Decimal places amounts are written with, at most. */
const DECIMALS = 2;
/** How often fuel is read, for a carried value's note. */
const CADENCE = 'fuel is read once a day.';

const nullableText = (value: unknown) => value === null || typeof value === 'string';
const raws = (value: unknown, length: number) => Array.isArray(value) && value.length === length && value.every(isRaw);
const countryReference = (value: unknown) => unsigned64(value) && BigInt(value) < (1n << 32n);
const span = (value: Record<string, unknown>) => signed32(value.firstHour) && signed32(value.lastHour) && value.firstHour <= value.lastHour;
const isAwards = (value: unknown): value is FuelAwardTotals => object(value) && raws(value.gainsRaw, 4)
  && isRaw(value.ownGainRaw) && isRaw(value.ownGainQuantityRaw) && raws(value.transfersRaw, 4)
  && isRaw(value.consumedRaw) && raws(value.receivedRaw, 3);

const isRead = (value: unknown): value is FuelRead => object(value) && span(value) && typeof value.startsSegment === 'boolean'
  && countryReference(value.country) && (value.countryInstance === null || unsigned32(value.countryInstance))
  && (value.continuityStartHour === null || signed32(value.continuityStartHour)) && nullableText(value.awardsReason)
  && isRaw(value.fuelRaw) && isRaw(value.capacityRaw) && isRaw(value.lendLeaseRaw)
  // A read has its day, or says why it is left out. Applied operands must agree with the authoritative daily totals.
  && (value.day === null ? typeof value.leftOut === 'string' && Object.hasOwn(LEFT_OUT, value.leftOut)
    : value.leftOut === null && object(value.day) && isRaw(value.day.gainedRaw) && isRaw(value.day.burnedRaw)
      && isRaw(value.day.otherRaw) && raws(value.day.receivedRaw, 3)
      && (value.day.awards === null ? typeof value.awardsReason === 'string'
        : isAwards(value.day.awards) && value.countryInstance !== null && value.continuityStartHour !== null && value.awardsReason === null
          && value.day.awards.ownGainRaw === value.day.gainedRaw && value.day.awards.consumedRaw === value.day.burnedRaw
          && value.day.awards.transfersRaw[3] === value.day.otherRaw
          && value.day.awards.receivedRaw.every((raw, i) => raw === (value.day as FuelRead['day'])!.receivedRaw[i])));

/** The lazy recipient reply retains the native identity and each refused name beside its exact amount. */
export function validateFuelAwardsReply(data: unknown): FuelAwardsReply {
  const reply = data as FuelAwardsReply;
  if (!object(data) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !signed32(reply.hour)
    || !object(reply.awards) || !isAwards(reply.awards.totals) || !Array.isArray(reply.awards.recipients)
    || !reply.awards.recipients.every(part => object(part) && span(part) && unsigned32(part.kind) && part.kind <= 2
      && unsigned64(part.reference) && countryReference(part.owner) && unsigned64(part.host) && isRaw(part.amountRaw)
      && typeof part.key === 'string' && part.key.length > 0 && nullableText(part.recordedName) && nullableText(part.nameReason))
    || new Set(reply.awards.recipients.map(part => part.key)).size !== reply.awards.recipients.length
    || !Array.isArray(reply.warnings) || !reply.warnings.every(note => typeof note === 'string')) {
    throw new Error('The fuel recipient reply is malformed.');
  }
  return reply;
}

/** A selected native recipient has exact held daily reads or explicit unknowns. */
export function validateFuelAwardHistory(data: unknown): FuelAwardHistoryReply {
  const reply = data as FuelAwardHistoryReply;
  if (!object(data) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !signed32(reply.hour)
    || !countryReference(reply.country) || !unsigned32(reply.countryInstance) || !signed32(reply.continuityStartHour) || !signed32(reply.window)
    || typeof reply.recipientKey !== 'string' || !reply.recipientKey || !span(reply as unknown as Record<string, unknown>)
    || !Array.isArray(reply.runs) || !reply.runs.every((run, i) => object(run) && span(run) && typeof run.startsSegment === 'boolean'
      && (run.amountRaw === null ? typeof run.reason === 'string' : isRaw(run.amountRaw) && run.reason === null)
      && (i === 0 || reply.runs[i - 1].lastHour < run.firstHour))
    || !Array.isArray(reply.warnings) || !reply.warnings.every(note => typeof note === 'string')) {
    throw new Error('The fuel recipient history is malformed.');
  }
  return reply;
}

const isLabel = (value: unknown) => object(value) && typeof value.key === 'string' && nullableText(value.template)
  && Array.isArray(value.arguments) && value.arguments.every(arg => arg === 'NUM' || arg === 'OIL') && nullableText(value.reason);

/** Checks the reply's shape; reads must be in hour order and never overlap. */
export function validateFuelReply(data: unknown): FuelReply {
  const reply = data as FuelReply;
  const scale = (value: unknown) => unsigned32(value) && value > 0;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && scale(reply.quantityScale) && scale(reply.gainScale) && scale(reply.dayHours) && scale(reply.detailHours)
    && Array.isArray(reply.gainLabels) && reply.gainLabels.length === 4 && reply.gainLabels.every(isLabel)
    && Array.isArray(reply.transferLabels) && reply.transferLabels.length === 3 && reply.transferLabels.every(isLabel)
    && Array.isArray(reply.consumerNames) && reply.consumerNames.length === 3 && reply.consumerNames.every(name => name === null || typeof name === 'string')
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.reads) && reply.reads.every(isRead)
    && reply.reads.every((read, index) => index === 0 || reply.reads[index - 1].lastHour < read.firstHour);
  if (!ok) throw new Error('The fuel reply is malformed.');
  return reply;
}

/** What a fuel widget shows of a reply inside [from, to]: the reads cut to it (for
 * lines and lookups), the hours they were read (for dating and keyboard stops), and
 * each kept day as a bar from `dayHours` before its read to the read: gained up,
 * burned down. A day is known only at its read, so a bar shows only when its read
 * is inside, cut to the range's start. */
export function fuelView(reply: FuelReply, from: number, to: number) {
  const range = [{ firstHour: from, lastHour: to }];
  const bars: StripBar[] = reply.reads.flatMap(read => read.day && read.firstHour >= from && read.firstHour <= to
    ? [{ firstHour: read.firstHour - reply.dayHours, lastHour: read.firstHour,
      up: Number(read.day.gainedRaw) / reply.gainScale, down: Number(read.day.burnedRaw) / reply.quantityScale }] : []);
  return { reads: clipToSpans(reply.reads, range), readHours: reply.reads.map(read => read.firstHour), bars: clipToSpans(bars, range) };
}

/** Runs of the tank's level or its capacity, as plotting numbers; labels use the exact raws. */
export function fuelRuns(reads: readonly FuelRead[], scale: number, of: 'fuel' | 'capacity'): Run[] {
  return reads.map(read => ({ firstHour: read.firstHour, lastHour: read.lastHour, value: Number(of === 'fuel' ? read.fuelRaw : read.capacityRaw) / scale,
    breakBefore: read.startsSegment }));
}

/** A recorded fuel quantity, exact to `digits` decimals (none when whole). */
export const formatFuel = (raw: string, scale: number, digits = DECIMALS) => formatAmount(rescale(raw, scale, digits), digits, digits);

/** Exact finite decimal fuel amounts; the common denominator may retain more
 * places than either native scale alone. No plotting number supplies the text. */
export function formatFuelPart(raw: bigint, scale: bigint): string {
  let denominator = scale, twos = 0, fives = 0;
  while (denominator % 2n === 0n) { denominator /= 2n; twos++; }
  while (denominator % 5n === 0n) { denominator /= 5n; fives++; }
  if (denominator !== 1n) return `${raw}/${scale}`;
  const places = Math.max(twos, fives);
  return formatAmount(rescale(raw, scale, places), places, places);
}

export const fuelPartShare = (raw: bigint, total: bigint) => total === 0n ? '\u2014' : `${formatAmount(rescale(raw * (total < 0n ? -100n : 100n), total < 0n ? -total : total, 1), 1, 1)}%`;

/** The day before a read in hundredths of fuel: gained, burned and the net, each
 * rounded once from the recorded values, never from the rounded parts. */
export function dayAmounts(reply: Pick<FuelReply, 'quantityScale' | 'gainScale'>, read: Pick<FuelRead, 'day'>) {
  if (!read.day) return null;
  const [gainScale, quantityScale] = [BigInt(reply.gainScale), BigInt(reply.quantityScale)];
  const gained = BigInt(read.day.gainedRaw);
  const burned = BigInt(read.day.burnedRaw);
  return {
    gained: rescale(gained, gainScale, DECIMALS),
    burned: rescale(burned, quantityScale, DECIMALS),
    net: read.day.awards
      ? rescale(BigInt(read.day.awards.ownGainQuantityRaw) - burned, quantityScale, DECIMALS)
      : rescale(gained * quantityScale - burned * gainScale, gainScale * quantityScale, DECIMALS),
  };
}

/** How full the tank is, as a whole percent; null without a capacity. */
export function percentFull(read: Pick<FuelRead, 'fuelRaw' | 'capacityRaw'>): bigint | null {
  const capacity = BigInt(read.capacityRaw);
  return capacity > 0n ? rescale(BigInt(read.fuelRaw) * 100n, capacity, 0) : null;
}

export interface FuelPart { id: string; label: string; raw: bigint; consumer?: number; muted?: boolean; reason?: string | null }
export type FuelPartLabels = Pick<FuelReply, 'gainLabels' | 'transferLabels'>;

/** Fuel labels use NUM and, for oil, OIL placeholders. The recorded catalogue
 * has already expanded references and removed markup. */
export function fuelLabel(label: FuelLabel | undefined, raw: bigint, scale: bigint): string | null {
  return label?.template?.replaceAll('{NUM}', formatFuelPart(raw, scale)).replaceAll('{OIL}', '?') ?? null;
}

/** Common-denominator operands retain each hourly conversion residual. Incoming
 * lend-lease is separate from own production and its share denominator. */
export function fuelParts(reply: FuelReply, read: FuelRead, measure: 'gain' | 'burn', labels: FuelPartLabels = reply):
  { scale: bigint; total: bigint; parts: FuelPart[]; transfer: FuelPart | null; reason: string | null } | null {
  if (!read.day) return null;
  const gain = BigInt(reply.gainScale), quantity = BigInt(reply.quantityScale), awards = read.day.awards;
  const named = (id: string, raw: bigint, label: string | null | undefined, reason?: string | null): FuelPart => ({ id, raw,
    label: label ?? 'Other / not identified', muted: label == null, reason });
  if (measure === 'gain') return { scale: gain, total: BigInt(read.day.gainedRaw),
    parts: awards ? awards.gainsRaw.map((raw, i) => named(`gain-${i}`, BigInt(raw), fuelLabel(labels.gainLabels[i], BigInt(raw), gain), labels.gainLabels[i].reason))
      : [{ id: 'unidentified', label: 'Other / not identified', raw: BigInt(read.day.gainedRaw), muted: true }],
    transfer: awards ? named('incoming', BigInt(awards.transfersRaw[0]), fuelLabel(labels.transferLabels[0], BigInt(awards.transfersRaw[0]), quantity), labels.transferLabels[0].reason) : null,
    reason: awards ? null : read.awardsReason };
  const total = BigInt(read.day.burnedRaw) * gain;
  const received = read.day.receivedRaw.map((raw, index) => ({ ...named(`consumer-${index}`, BigInt(raw) * quantity, reply.consumerNames[index]), consumer: index }));
  const other = BigInt(read.day.otherRaw) * gain;
  const residual = total - other - received.reduce((sum, part) => sum + part.raw, 0n);
  const transfers: FuelPart[] = awards ? [named('outgoing', BigInt(awards.transfersRaw[1]) * gain, fuelLabel(labels.transferLabels[1], BigInt(awards.transfersRaw[1]), quantity), labels.transferLabels[1].reason),
    named('flat', BigInt(awards.transfersRaw[2]) * gain, fuelLabel(labels.transferLabels[2], BigInt(awards.transfersRaw[2]), quantity), labels.transferLabels[2].reason),
    { id: 'other', label: 'Other / not identified', raw: other - (BigInt(awards.transfersRaw[1]) + BigInt(awards.transfersRaw[2])) * gain, muted: true }]
    : [{ id: 'other', label: 'Other / not identified', raw: other, muted: true }];
  return { scale: gain * quantity, total, parts: [...received, ...transfers,
    { id: 'conversion', label: 'Hourly conversion difference', raw: residual }], transfer: null, reason: awards ? null : read.awardsReason };
}

/** Missing source rows never borrow the separate transfer's label or amount. */
export const fuelHistoryRow = (values: ReturnType<typeof fuelParts>, id: string, transfer: boolean) =>
  values?.parts.find(part => part.id === id) ?? (transfer ? values?.transfer ?? null : null);

/** Current text and chart hover share membership and unknown-run semantics. */
export function fuelRecipientAmount(runs: readonly FuelAwardRun[], hour: number | null, within: readonly Span[] | null | undefined, gainScale: number) {
  const missing = (text: string) => ({ raw: null, value: null, text });
  if (hour === null) return missing('No selected moment');
  if (!isMember(within ?? undefined, hour)) return missing('Outside the selected group membership');
  const run = runAt(runs, hour);
  if (run?.amountRaw == null) return missing(run?.reason ?? 'Recipient history not loaded for this window');
  const raw = BigInt(run.amountRaw);
  return { raw, value: Number(raw) / gainScale, text: formatFuelPart(raw, BigInt(gainScale)) };
}

/** A part is absent when its attribution was not recorded, never a known zero.
 * Daily histories use stable source keys across legacy and actual accounting. */
export function fuelPartHistory(reply: FuelReply, measure: 'gain' | 'burn', labels: FuelPartLabels = reply) {
  const parts = new Map<string, { id: string; label: string; runs: Run[]; transfer: boolean }>();
  for (const read of reply.reads) {
    const values = fuelParts(reply, read, measure, labels);
    if (!values) continue;
    for (const part of [...values.parts, ...(values.transfer ? [values.transfer] : [])]) {
      const transfer = part === values.transfer;
      let series = parts.get(part.id);
      if (!series) { series = { id: part.id, label: part.label, runs: [], transfer }; parts.set(part.id, series); }
      const previous = series.runs.at(-1);
      series.runs.push({ firstHour: read.firstHour, lastHour: read.lastHour,
        breakBefore: read.startsSegment || !previous || previous.lastHour + 1 !== read.firstHour,
        value: Math.max(0, Number(part.raw) / Number(transfer ? BigInt(reply.quantityScale) : values.scale)) });
    }
  }
  return [...parts.values()];
}

/** Lazy requests always name the original supplying read, including at carried display hours. */
export function fuelAwardBasis(reply: Pick<FuelReply, 'branchId' | 'tag'>, read: FuelRead | null | undefined) {
  return read?.day?.awards && read.countryInstance !== null && read.continuityStartHour !== null ? { branchId: reply.branchId, tag: reply.tag,
    hour: read.firstHour, country: read.country, countryInstance: read.countryInstance } : null;
}

/** Full and compact tooltip forms: the tank, then the day before its read. */
export function fuelTooltips(reply: FuelReply, reads: readonly FuelRead[], readHours: readonly number[], hour: number, title: string,
  colors: { fuel: string; capacity: string; gained: string; burned: string }): TooltipContent[] {
  const read = runAt(reads, hour);
  if (!read) return [{ title, rows: [{ label: 'Not recorded at this time', tone: 'muted' }] }];
  const full = percentFull(read);
  const tank: TooltipRow[] = [
    { label: 'Fuel', value: formatFuel(read.fuelRaw, reply.quantityScale), swatch: colors.fuel },
    { label: 'Capacity', value: formatFuel(read.capacityRaw, reply.quantityScale), swatch: colors.capacity, tone: 'muted' },
    { label: 'Full', value: full == null ? 'No capacity' : `${full}%` },
  ];
  const notes = [readNote(readAt(readHours, hour), hour, CADENCE)];
  const day = dayAmounts(reply, read);
  let rows: TooltipRow[];
  if (day) {
    const net: TooltipRow = { label: 'Net', value: formatSigned(day.net, DECIMALS, DECIMALS), strong: true,
      tone: day.net < 0n ? 'negative' : day.net > 0n ? 'positive' : undefined };
    rows = [...tank, { label: 'Gained (own production)', value: formatSigned(day.gained, DECIMALS, DECIMALS), swatch: colors.gained, divider: true },
      { label: 'Burned', value: formatSigned(-day.burned, DECIMALS, DECIMALS), swatch: colors.burned }, net];
    notes.push(`Gained and burned in the ${reply.dayHours} hours before the read; lend-lease fuel received is not included.`);
    if (read.fuelRaw === read.capacityRaw && day.net > 0n) notes.push('The tank was full: the game discards fuel beyond its capacity.');
  } else {
    const why = LEFT_OUT[read.leftOut ?? 'notUpdated'](reply.dayHours);
    rows = [...tank, { label: 'Gained and burned', value: why.value, tone: 'muted', divider: true }];
    notes.push(why.note);
  }
  const incoming = read.day?.awards?.transfersRaw[0];
  if (day && incoming !== undefined) {
    rows.push({ label: 'Lend-lease received (separate)', value: formatSigned(rescale(incoming, reply.quantityScale, DECIMALS), DECIMALS, DECIMALS) });
    notes.push('The recorded daily lend-lease receipt is separate from own production and its net.');
  } else {
    const lendLease = BigInt(read.lendLeaseRaw);
    if (day && lendLease !== 0n) notes.push(`Lend-lease fuel was arriving at the read (${formatSigned(rescale(lendLease, reply.quantityScale, DECIMALS), DECIMALS, DECIMALS)} in its last hour); its daily total was not recorded.`);
  }
  const note = notes.filter(Boolean).join(' ');
  // In a short window the day's total stays, with the tank's level above it.
  return [{ title, rows, note }, { title, rows: [tank[0], rows.find(row => row.strong) ?? rows.at(-1)!], note }];
}

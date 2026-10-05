import type { DayLeftOut, FuelRead, FuelReply } from '../../generated/dashboard';
import { readAt, readNote } from '../../dashboard/reads.ts';
import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Run } from '../../charts/series';
import type { StripBar } from '../../charts/strip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatSigned, rescale } from '../../charts/valueAxis.ts';

const RAW = /^-?\d{1,20}$/;
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

const isRead = (value: unknown): value is FuelRead => {
  const read = value as FuelRead;
  return typeof read === 'object' && read != null && Number.isInteger(read.firstHour) && Number.isInteger(read.lastHour)
    && read.firstHour <= read.lastHour && typeof read.startsSegment === 'boolean' && RAW.test(read.fuelRaw) && RAW.test(read.capacityRaw)
    && RAW.test(read.lendLeaseRaw)
    // A read has its day, or says why it is left out.
    && (read.day == null
      ? typeof read.leftOut === 'string' && Object.hasOwn(LEFT_OUT, read.leftOut)
      : read.leftOut == null && typeof read.day === 'object' && RAW.test(read.day.gainedRaw) && RAW.test(read.day.burnedRaw));
};

/** Checks the reply's shape; reads must be in hour order and never overlap. */
export function validateFuelReply(data: unknown): FuelReply {
  const reply = data as FuelReply;
  const scale = (value: unknown) => Number.isSafeInteger(value) && (value as number) > 0;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && scale(reply.quantityScale) && scale(reply.gainScale) && scale(reply.dayHours)
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
    net: rescale(gained * quantityScale - burned * gainScale, gainScale * quantityScale, DECIMALS),
  };
}

/** How full the tank is, as a whole percent; null without a capacity. */
export function percentFull(read: Pick<FuelRead, 'fuelRaw' | 'capacityRaw'>): bigint | null {
  const capacity = BigInt(read.capacityRaw);
  return capacity > 0n ? rescale(BigInt(read.fuelRaw) * 100n, capacity, 0) : null;
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
  // Only the latest hour's lend-lease fuel is recorded: it is named, never summed into a
  // day, and only for a read whose day the game updated; a left-out read's value may be
  // frozen from before the country lost its states, so nothing is claimed from it.
  const lendLease = BigInt(read.lendLeaseRaw);
  if (day && lendLease !== 0n) {
    notes.push(`Lend-lease fuel was arriving at the read (${formatSigned(rescale(lendLease, reply.quantityScale, DECIMALS), DECIMALS, DECIMALS)} in its last hour); the tank gains it, but these figures leave it out.`);
  }
  const note = notes.filter(Boolean).join(' ');
  // In a short window the day's total stays, with the tank's level above it.
  return [{ title, rows, note }, { title, rows: [tank[0], rows.at(-1)!], note }];
}

import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedWindow } from '../../dashboard/keyedHistory';
import type { TooltipContent } from '../../charts/tooltip';

export interface AirChannel { id: string; ours: string; enemy: string; divisor: number }
export interface AirRegion { id: string; name: string }
export interface AirTheatre { id: string; name: string; regions: readonly string[] }
export interface AirStatisticsReply { window: KeyedWindow; channels: readonly AirChannel[]; regions: readonly AirRegion[]; theatres?: readonly AirTheatre[]; warnings: readonly string[] }

/** The channel the widget starts with: the recording schema's fighters-lost pair. */
export const DEFAULT_CHANNEL = '3';
/** The game's statistic names end in a colon, as they lead a number in its window. */
export const statisticName = (text: string) => text.replace(/\s*:\s*$/, '');
/** The schema's ours channels come first, then the same number of enemy channels. */
const PAIRS = 12;

const isChoice = (value: unknown): value is AirRegion => !!value && typeof value === 'object'
  && typeof (value as AirRegion).id === 'string' && typeof (value as AirRegion).name === 'string';

export function validateAirStatistics(value: unknown): AirStatisticsReply {
  const reply = value as AirStatisticsReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.channels) || !Array.isArray(reply.regions) || !Array.isArray(reply.warnings)
    || !reply.channels.every(channel => isChoice({ id: channel?.id, name: channel?.ours }) && typeof channel.enemy === 'string'
      && Number.isSafeInteger(channel.divisor) && channel.divisor > 0)
    || !reply.regions.every(isChoice)
    || reply.theatres !== undefined && !(Array.isArray(reply.theatres) && reply.theatres.every(theatre => isChoice(theatre) && Array.isArray((theatre as AirTheatre).regions)
      && (theatre as AirTheatre).regions.every((region: unknown) => typeof region === 'string'))) || !reply.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The air statistics reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

/** One region's pair for the chosen channel at an hour: recorded words, or null where the
 * region's record holds none. Regions without a record at the hour are left out. */
export interface RegionPair { region: string; name: string; ours: bigint | null; enemy: bigint | null }

/** Each region's recorded pair for `channel` at `hour`, for the country `tag`; null when the
 * statistics are not recorded then. A region absent from the record is absent, never zero. */
export function pairsAt(reply: AirStatisticsReply, tag: string, channel: string, hour: number): { rows: RegionPair[]; readHour: number | undefined } | null {
  if (!keyedKnown(reply.window, hour)) return null;
  const index = Number(channel);
  const names = new Map(reply.regions.map(region => [region.id, region.name]));
  const pairs = new Map<string, RegionPair>();
  for (const series of reply.window.series) {
    const [country, region, field] = series.name.split('|');
    if (countryPart(country)?.tag !== tag || !field?.startsWith('c')) continue;
    const at = Number(field.slice(1));
    if (at !== index && at !== index + PAIRS) continue;
    const run = runAtHour(series.runs, hour);
    if (!run) continue;
    const pair = pairs.get(region) ?? { region, name: names.get(region) ?? 'Region name unavailable', ours: null, enemy: null };
    if (at === index) pair.ours = BigInt(run[2]); else pair.enemy = BigInt(run[2]);
    pairs.set(region, pair);
  }
  return { rows: [...pairs.values()], readHour: keyedRead(reply.window, hour) };
}

const magnitude = (value: bigint | null) => value === null ? 0n : value < 0n ? -value : value;

/** Regions ordered by ours plus enemy, largest first, ties by name; `regions` narrows them and
 * `top` keeps that many (none kept back when null). */
export function rankedPairs(rows: readonly RegionPair[], regions: ReadonlySet<string> | null, top: number | null): RegionPair[] {
  const kept = rows.filter(row => !regions || regions.has(row.region))
    .sort((a, b) => {
      const difference = magnitude(b.ours) + magnitude(b.enemy) - (magnitude(a.ours) + magnitude(a.enemy));
      return difference > 0n ? 1 : difference < 0n ? -1 : a.name.localeCompare(b.name) || a.region.localeCompare(b.region);
    });
  return top === null ? kept : kept.slice(0, top);
}

/** A recorded word on its channel's display scale, exactly (no rounding beyond the scale). */
export function scaled(raw: bigint, divisor: number): string {
  const scale = BigInt(divisor), negative = raw < 0n, whole = (negative ? -raw : raw) / scale, part = (negative ? -raw : raw) % scale;
  const digits = String(divisor).length - 1;
  const fraction = part === 0n ? '' : `.${part.toString().padStart(digits, '0').replace(/0+$/u, '')}`;
  return `${negative ? '-' : ''}${whole.toLocaleString('en-US')}${fraction}`;
}

/** The chart operand: the scaled value as a number, for bar length only. */
export const barValue = (raw: bigint | null, divisor: number) => raw === null ? null : Number(raw) / divisor;

/** Enemy per ours, when both are recorded and ours is positive. */
export function pairRatio(row: RegionPair, divisor: number): string {
  if (row.ours === null || row.enemy === null || row.ours <= 0n) return '—';
  return (Number(row.enemy) / divisor / (Number(row.ours) / divisor)).toLocaleString('en-US', { maximumFractionDigits: 2 });
}

export function pairTooltip(row: RegionPair, channel: AirChannel, readDate: string | null): TooltipContent {
  return {
    title: row.name,
    rows: [
      { label: statisticName(channel.ours), value: row.ours === null ? 'Not recorded' : scaled(row.ours, channel.divisor) },
      { label: statisticName(channel.enemy), value: row.enemy === null ? 'Not recorded' : scaled(row.enemy, channel.divisor) },
      { label: 'Enemy per ours', value: pairRatio(row, channel.divisor) },
      { label: 'Window', value: 'Current year window (twelve 30-day periods)' },
      ...readDate ? [{ label: 'As of', value: readDate }] : [],
    ],
  };
}

/** The regions of the chosen theatres; null keeps every region. */
export function theatreRegions(theatres: readonly AirTheatre[] | undefined, chosen: readonly string[]): Set<string> | null {
  if (!chosen.length) return null;
  return new Set((theatres ?? []).filter(theatre => chosen.includes(theatre.id)).flatMap(theatre => theatre.regions));
}

import type { TooltipContent } from '../../charts/tooltip';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { AirStatisticsChannel, AirStatisticsReply } from '../../generated/dashboard';

export type { AirStatisticsReply };

// A region's two words are its live cells (`region`, `ours`) and (`region`, `enemy`), never in the reply.

/** The channel the widget starts with: the recording schema's fighters-lost pair. */
export const DEFAULT_CHANNEL = '3';
/** The schema's ours channels come first, then the same number of enemy channels. */
export const PAIRS = 12;
/** The game's statistic names end in a colon, as they lead a number in its window. */
export const statisticName = (text: string) => text.replace(/\s*:\s*$/, '');

const text = (value: unknown): value is string => typeof value === 'string';
const optional = (value: unknown) => value === null || text(value);

export function validateAirStatistics(value: unknown): AirStatisticsReply {
  const reply = value as AirStatisticsReply;
  if (!reply || typeof reply !== 'object' || !(reply.readHour === null || Number.isSafeInteger(reply.readHour)) || !optional(reply.missing)
    || !Array.isArray(reply.channels) || !Array.isArray(reply.regions) || !Array.isArray(reply.warnings) || !reply.warnings.every(text)
    || !reply.channels.every(channel => channel && text(channel.id) && text(channel.ours) && text(channel.enemy) && Number.isSafeInteger(channel.divisor) && channel.divisor > 0)
    || !reply.regions.every(row => row && text(row.region) && optional(row.name) && optional(row.missing) && (row.name === null) !== (row.missing === null))
    || new Set(reply.regions.map(row => row.region)).size !== reply.regions.length || reply.missing !== null && reply.regions.length > 0
    || !optional(reply.theatresMissing) || !(reply.theatres === null || Array.isArray(reply.theatres)
      && reply.theatres.every(theatre => theatre && text(theatre.id) && text(theatre.name) && Array.isArray(theatre.regions) && theatre.regions.every(text))))
    throw new Error('The air statistics reply is malformed.');
  return reply;
}

/** A recorded pair channel as its request names it; null for one the schema has not. */
export function channelIndex(channel: string): number | null {
  const index = Number(channel);
  return Number.isSafeInteger(index) && index >= 0 && index < PAIRS ? index : null;
}

/** One region's pair: its recorded words, exact; a word not known is null, `why` saying why. */
export interface RegionPair { region: string; name: string; ours: bigint | null; enemy: bigint | null; oursWhy: string; enemyWhy: string }

const word = (values: LiveValues<number>, key: string) => { const value = values.get(key); return value === undefined ? null : BigInt(Math.trunc(value)); };

/** The reply's regions with their words from the live cells. */
export const pairsOf = (reply: Pick<AirStatisticsReply, 'regions'>, values: LiveValues<number>): RegionPair[] => reply.regions.map(row => {
  const ours = liveKey(row.region, 'ours'), enemy = liveKey(row.region, 'enemy');
  return { region: row.region, name: row.name ?? `Region ${row.region} (${row.missing})`, ours: word(values, ours), enemy: word(values, enemy),
    oursWhy: values.missing(ours), enemyWhy: values.missing(enemy) };
});

const magnitude = (value: bigint | null) => value === null ? 0n : value < 0n ? -value : value;

/** Regions ordered by ours plus enemy, largest first, ties by name; `regions` narrows them and `top` keeps that many
 * (none kept back when null). */
export function rankedPairs(rows: readonly RegionPair[], regions: ReadonlySet<string> | null, top: number | null): RegionPair[] {
  const kept = rows.filter(row => !regions || regions.has(row.region))
    .sort((a, b) => {
      const difference = magnitude(b.ours) + magnitude(b.enemy) - (magnitude(a.ours) + magnitude(a.enemy));
      return difference > 0n ? 1 : difference < 0n ? -1 : a.name.localeCompare(b.name) || a.region.localeCompare(b.region);
    });
  return top === null ? kept : kept.slice(0, top);
}

/** A recorded word on its channel's display scale, exactly (no rounding beyond the scale); a dash when not known. */
export function scaled(raw: bigint | null, divisor: number): string {
  if (raw === null) return '—';
  const scale = BigInt(divisor), negative = raw < 0n, whole = (negative ? -raw : raw) / scale, part = (negative ? -raw : raw) % scale;
  const digits = String(divisor).length - 1;
  const fraction = part === 0n ? '' : `.${part.toString().padStart(digits, '0').replace(/0+$/u, '')}`;
  return `${negative ? '-' : ''}${whole.toLocaleString('en-US')}${fraction}`;
}

/** The chart operand: the scaled value as a number, for bar length only. */
export const barValue = (raw: bigint | null, divisor: number) => raw === null ? 0 : Number(raw) / divisor;

/** Enemy per ours, when ours is positive. */
export function pairRatio(row: RegionPair, divisor: number): string {
  if (row.ours === null || row.enemy === null || row.ours <= 0n) return '—';
  return (Number(row.enemy) / divisor / (Number(row.ours) / divisor)).toLocaleString('en-US', { maximumFractionDigits: 2 });
}

export function pairTooltip(row: RegionPair, channel: AirStatisticsChannel, readDate: string | null): TooltipContent {
  return {
    title: row.name,
    rows: [
      { label: statisticName(channel.ours), value: row.ours === null ? row.oursWhy : scaled(row.ours, channel.divisor) },
      { label: statisticName(channel.enemy), value: row.enemy === null ? row.enemyWhy : scaled(row.enemy, channel.divisor) },
      { label: 'Enemy per ours', value: pairRatio(row, channel.divisor) },
      { label: 'Window', value: 'Current year window (twelve 30-day periods)' },
      ...readDate ? [{ label: 'As of', value: readDate }] : [],
    ],
  };
}

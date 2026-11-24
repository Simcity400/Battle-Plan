import type { AirStatisticsReply } from '../../generated/dashboard';

export type { AirStatisticsReply };


export const DEFAULT_CHANNEL = '3';
export const PAIRS = 12;
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

export function channelIndex(channel: string): number | null {
  const index = Number(channel);
  return Number.isSafeInteger(index) && index >= 0 && index < PAIRS ? index : null;
}

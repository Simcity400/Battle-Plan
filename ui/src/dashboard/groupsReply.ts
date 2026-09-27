import type { CountryGroupReply } from '../generated/dashboard';

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null;
const isHour = (value: unknown) => Number.isInteger(value);
const isSpan = (value: unknown) => isObject(value) && isHour(value.firstHour) && isHour(value.lastHour) && (value.firstHour as number) <= (value.lastHour as number);
const isRun = (value: unknown) => isSpan(value) && typeof (value as Record<string, unknown>).tag === 'string';
const listOf = (value: unknown, check: (item: unknown) => boolean) => Array.isArray(value) && value.every(check);

/** Checks a `country-groups` reply's shape: every membership, war, war start and
 * capitulation list, and the unobserved spans, so a reply from another version is
 * refused rather than read wrongly. */
export function validateCountryGroupReply(data: unknown): CountryGroupReply {
  const reply = data as Record<string, unknown>;
  const unobserved = isObject(reply) ? reply.unobserved : null;
  const ok = isObject(reply) && typeof reply.branchId === 'string'
    && listOf(reply.warnings, warning => typeof warning === 'string')
    && listOf(reply.humans, isRun)
    && listOf(reply.factions, faction => isSpan(faction) && isObject(faction) && typeof faction.id === 'string'
      && (faction.name === null || typeof faction.name === 'string') && listOf(faction.members, isRun) && listOf(faction.incomplete, isSpan))
    && listOf(reply.wars, war => isSpan(war) && isObject(war) && typeof war.first === 'string' && typeof war.second === 'string')
    && listOf(reply.warStarts, start => isObject(start) && isHour(start.hour) && typeof start.attacking === 'string' && typeof start.defending === 'string')
    && listOf(reply.capitulations, event => isObject(event) && isHour(event.hour) && typeof event.tag === 'string')
    && listOf(reply.identityChanges, change => isObject(change) && isHour(change.hour) && typeof change.from === 'string' && typeof change.to === 'string')
    && isObject(unobserved) && listOf(unobserved.humans, isSpan) && listOf(unobserved.factions, isSpan) && listOf(unobserved.wars, isSpan);
  if (!ok) throw new Error('The country groups reply is malformed.');
  return data as CountryGroupReply;
}

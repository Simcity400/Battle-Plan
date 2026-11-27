import type { CountryGroupReply } from '../generated/dashboard';
import { isOrderedSpans, isSpan } from '../charts/series.ts';
import { validCountryIdentityHistories } from './countryIdentity.ts';

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null;
const isHour = (value: unknown) => Number.isInteger(value);
const isRun = (value: unknown) => isSpan(value) && typeof (value as Record<string, unknown>).tag === 'string';
const isName = (value: unknown) => isObject(value) && ((typeof value.name === 'string' && value.reason === null)
  || (value.name === null && typeof value.reason === 'string'));
const listOf = (value: unknown, check: (item: unknown) => boolean) => Array.isArray(value) && value.every(check);
const isWarSide = (tag: unknown, reason: unknown) => typeof tag === 'string' && reason === null
  || tag === null && typeof reason === 'string' && reason.length > 0;

export const groupNumbers = (reply: CountryGroupReply): CountryGroupReply => ({ ...reply, warnings: [],
  factions: reply.factions.map(faction => ({ ...faction, name: null, nameReason: null, names: [] })) });

export function validateCountryGroupReply(data: unknown): CountryGroupReply {
  const reply = data as Record<string, unknown>;
  const unobserved = isObject(reply) ? reply.unobserved : null;
  const ok = isObject(reply) && typeof reply.branchId === 'string'
    && listOf(reply.warnings, warning => typeof warning === 'string')
    && listOf(reply.humans, isRun)
    && listOf(reply.factions, faction => isSpan(faction) && isObject(faction) && typeof faction.id === 'string'
      && isName({ name: faction.name, reason: faction.nameReason }) && isOrderedSpans(faction.names) && faction.names.every(isName)
      && typeof faction.recordedName === 'string' && listOf(faction.members, isRun) && listOf(faction.leaders, isRun) && listOf(faction.incomplete, isSpan))
    && listOf(reply.wars, war => isSpan(war) && isObject(war) && typeof war.first === 'string' && typeof war.second === 'string')
    && listOf(reply.warStarts, start => isObject(start) && isHour(start.hour)
      && isWarSide(start.attacking, start.attackingMissing) && isWarSide(start.defending, start.defendingMissing))
    && listOf(reply.capitulations, event => isObject(event) && isHour(event.hour) && typeof event.tag === 'string')
    && listOf(reply.identityChanges, change => isObject(change) && isHour(change.hour) && typeof change.from === 'string' && typeof change.to === 'string')
    && validCountryIdentityHistories(reply.countryIdentities)
    && isObject(unobserved) && listOf(unobserved.humans, isSpan) && listOf(unobserved.factions, isSpan) && listOf(unobserved.wars, isSpan);
  if (!ok || 'approximate' in reply && !isOrderedSpans(reply.approximate)
    || 'eventsApproximate' in reply && !isOrderedSpans(reply.eventsApproximate)) throw new Error('The country groups reply is malformed.');
  return data as CountryGroupReply;
}

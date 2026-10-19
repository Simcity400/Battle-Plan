import type { PeopleFact, PeopleReply } from '../../generated/dashboard';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { formatNativeFixed5 } from '../../charts/valueAxis.ts';

export function peopleValue(fact: Pick<PeopleFact, 'value' | 'unit'>): string {
  if (fact.value === null) return 'Not recorded';
  if (fact.unit === 'text') return fact.value;
  if (fact.unit === 'date') return dayLabel(Number(fact.value));
  const raw = BigInt(fact.value);
  if (fact.unit === 'ratio' || fact.unit === 'ratio100' || fact.unit === 'dailyRatio') {
    const percent = fact.unit === 'ratio100' ? raw : BigInt.asIntN(64, raw * 100n);
    return `${formatNativeFixed5(percent, fact.unit === 'dailyRatio' ? 3 : 2)}%`;
  }
  return `${usNumber.format(raw)}${fact.unit === 'months' ? ' months' : ''}`;
}
export function validatePeople(value: unknown): PeopleReply {
  const fail = (): never => { throw new Error('Invalid country manpower and politics reply'); };
  if (!value || typeof value !== 'object') return fail();
  const reply = value as PeopleReply;
  const integer = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value);
  const optional = (value: unknown) => value === null || integer(value);
  const raw = (value: unknown) => value === null || typeof value === 'string' && /^-?\d+$/.test(value);
  if (typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !integer(reply.hour) || !optional(reply.readHour)
    || !Array.isArray(reply.warnings) || reply.warnings.some(value => typeof value !== 'string')) return fail();
  for (const list of [reply.manpower, reply.politics]) {
    if (!Array.isArray(list) || list.some(fact => !fact || typeof fact.label !== 'string'
      || !['text', 'people', 'months', 'date', 'ratio', 'ratio100', 'dailyRatio'].includes(fact.unit)
      || (fact.unit === 'text' ? fact.value !== null && typeof fact.value !== 'string' : !raw(fact.value)))) return fail();
  }
  if (!Array.isArray(reply.states) || reply.states.some(state => !state || !integer(state.state) || !integer(state.available) || !optional(state.locked) || !optional(state.total) || !raw(state.effective) || !optional(state.recruitable))) return fail();
  if (!Array.isArray(reply.exiles) || reply.exiles.some(exile => !exile || typeof exile.tag !== 'string' || exile.host !== null && typeof exile.host !== 'string'
    || !raw(exile.legitimacy) || ![exile.manpower, exile.cap, exile.leaders, exile.deployedAir, exile.readHour].every(optional))) return fail();
  return reply;
}

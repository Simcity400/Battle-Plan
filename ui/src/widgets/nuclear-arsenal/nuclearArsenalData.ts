import type { MeasureParts } from '../../charts/race/measureParts';
import type { NuclearArsenalReply, NukeType } from '../../generated/dashboard';

const integer = Number.isSafeInteger;

export function validateNuclearArsenal(value: unknown): NuclearArsenalReply {
  const reply = value as Partial<NuclearArsenalReply> | null;
  const ok = !!reply && typeof reply === 'object' && Array.isArray(reply.warnings)
    && (reply.missing === null || typeof reply.missing === 'string')
    && Array.isArray(reply.types) && reply.types.every(type => type && integer(type.ordinal) && (type.token === null || typeof type.token === 'string')
      && (type.name === null || typeof type.name === 'string'))
    && Array.isArray(reply.countries) && reply.countries.every(country => country && typeof country.tag === 'string' && integer(country.countryId) && integer(country.total)
      && Array.isArray(country.counts) && country.counts.every(count => count && integer(count.ordinal) && integer(count.value) && integer(count.ready)));
  if (!ok) throw new Error('The nuclear arsenal reply is malformed.');
  return reply as NuclearArsenalReply;
}

export const typeName = (type: NukeType) => type.name ?? type.token ?? `Unnamed nuclear type ${type.ordinal}`;

export function partName(reply: NuclearArsenalReply | undefined, part: string): string {
  const type = reply?.types.find(held => String(held.ordinal) === part);
  return type ? typeName(type) : `Unnamed nuclear type ${part}`;
}

export const nukeParts = (reply: NuclearArsenalReply | undefined): MeasureParts => ({ kind: 'keyed', label: 'Weapon types',
  family: { kind: 'country-nukes', measure: 'amount' }, partBits: 32, decimals: 0, name: part => partName(reply, part) });

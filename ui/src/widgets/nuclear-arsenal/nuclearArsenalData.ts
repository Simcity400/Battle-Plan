/** The Nuclear arsenal widget's data: each country's stockpile as the top bar shows it is the ranked `country-nuke-totals`,
 * each weapon type a part of it (`country-nukes`), the types named by the `nuclear-arsenal` reply. */
import type { MeasureParts } from '../../charts/race/measureParts';
import type { NuclearArsenalReply, NukeType } from '../../generated/dashboard';


const integer = Number.isSafeInteger;

export function validateNuclearArsenal(value: unknown): NuclearArsenalReply {
  const reply = value as Partial<NuclearArsenalReply> | null;
  const ok = !!reply && typeof reply === 'object' && integer(reply.hour) && Array.isArray(reply.warnings)
    && (reply.missing === null || typeof reply.missing === 'string')
    && Array.isArray(reply.types) && reply.types.every(type => type && integer(type.ordinal) && (type.token === null || typeof type.token === 'string')
      && (type.name === null || typeof type.name === 'string'))
    && Array.isArray(reply.countries) && reply.countries.every(country => country && typeof country.tag === 'string' && integer(country.total)
      && Array.isArray(country.counts) && country.counts.every(count => count && integer(count.ordinal) && integer(count.value) && integer(count.ready)));
  if (!ok) throw new Error('The nuclear arsenal reply is malformed.');
  return reply as NuclearArsenalReply;
}

/** A weapon type's name: its localised name, its token, or its ordinal when the game names none. */
export const typeName = (type: NukeType) => type.name ?? type.token ?? `Unnamed nuclear type ${type.ordinal}`;

/** A weapon type's name by its ordinal (a part's key), from the types the reply lists. */
export function partName(reply: NuclearArsenalReply | undefined, part: string): string {
  const type = reply?.types.find(held => String(held.ordinal) === part);
  return type ? typeName(type) : `Unnamed nuclear type ${part}`;
}

/** A stockpile's parts: each weapon type the country holds, a part of the sub-keyed `country-nukes` series. */
export const nukeParts = (reply: NuclearArsenalReply | undefined): MeasureParts => ({ kind: 'keyed', label: 'Weapon types',
  family: { kind: 'country-nukes', measure: 'amount' }, partBits: 32, decimals: 0, name: part => partName(reply, part) });
import { countryKey } from '../../dashboard/countryKey.ts';
import { forceAtSources } from '../../families/bars/forceAtRace.ts';
import { countryRowPresentation, type CountryRowPresentation } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { NuclearArsenalReply, NukeCountry, NukeType } from '../../generated/dashboard';

const integer = Number.isSafeInteger;

export function validateNuclearArsenal(value: unknown): NuclearArsenalReply {
  const reply = value as Partial<NuclearArsenalReply> | null;
  const ok = !!reply && typeof reply === 'object' && Array.isArray(reply.warnings)
    && (reply.missing === null || typeof reply.missing === 'string')
    && Array.isArray(reply.types) && reply.types.every(type => type && integer(type.ordinal) && (type.token === null || typeof type.token === 'string')
      && (type.name === null || typeof type.name === 'string'))
    && Array.isArray(reply.countries) && reply.countries.every(country => country && typeof country.tag === 'string' && integer(country.countryId)
      && (country.slot === null || integer(country.slot)) && integer(country.total)
      && Array.isArray(country.counts) && country.counts.every(count => count && integer(count.ordinal) && integer(count.value) && integer(count.ready)));
  if (!ok) throw new Error('The nuclear arsenal reply is malformed.');
  return reply as NuclearArsenalReply;
}

export const typeName = (type: NukeType) => type.name ?? type.token ?? `Unnamed nuclear type ${type.ordinal}`;

export function nuclearArsenalSources(ctx: FamilyPlanContext, view: 'race' | 'mix'): SourcePlan {
  const owner = `nuclear-arsenal:${view}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`;
  return forceAtSources(ctx, owner, 'nuclear-arsenal', {}, validateNuclearArsenal);
}

export function nukeRowIdentity(readView: BoundedReadView, row: NukeCountry): { readonly key: string; readonly pres: CountryRowPresentation } {
  const key = row.slot === null ? row.tag : countryKey(row.slot);
  const pres = countryRowPresentation(readView, key);
  return { key, pres };
}

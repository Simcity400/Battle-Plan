import type { MeasureParts } from '../../charts/race/measureParts';
import type { NuclearArsenalReply, NukeType } from '../../generated/dashboard';
import { dayLabel } from '../../charts/draw.ts';

export function arsenalNotice(reply: NuclearArsenalReply, displayedHour: number) {
  return reply.missing ?? 'No nuclear stockpile recorded on ' + dayLabel(displayedHour) + '.';
}

import { isMember } from '../../dashboard/groups.ts';
import type { HourSpan } from '../../generated/dashboard';
import type { CountryRaceEntry } from '../../charts/race/countryRace';

export interface ArsenalSelection {
  top: number | null; tags: readonly string[]; split: boolean; membership?: ReadonlyMap<string, readonly HourSpan[]> | null;
}

export function arsenalRows(reply: NuclearArsenalReply, hour: number, selection: ArsenalSelection): CountryRaceEntry[] {
  if (reply.missing) return [];
  const chosen = new Set(selection.tags);
  const rows = reply.countries.filter(row => (selection.top !== null ? row.total > 0 : chosen.has(row.tag))
    && (!selection.membership || isMember(selection.membership.get(row.tag), hour)));
  return rows.slice(0, selection.top ?? rows.length).map(row => ({ tag: row.tag, value: row.total, label: String(row.total), hour, stretch: null,
    ...(selection.split && row.counts.length ? { parts: row.counts.map(part => ({ id: String(part.ordinal), value: part.value })) } : {}) }));
}

export function arsenalStructureRows(reply: NuclearArsenalReply): string[] {
  const countries = new Map<number, [string, boolean, number[]]>();
  for (const row of reply.countries) countries.set(row.countryId, [row.tag, row.total > 0, row.counts.map(v => v.ordinal)]);
  return [
    'missing:' + String(reply.missing),
    ...reply.types.map(type => 'type:' + JSON.stringify([type.ordinal, type.token, type.name])),
    ...reply.warnings.map(warning => 'warning:' + JSON.stringify(warning)),
    ...[...countries].sort(([a], [b]) => a - b).map(([countryId, values]) => `country:${JSON.stringify([countryId, ...values])}`),
  ];
}

export function arsenalStructure(reply: NuclearArsenalReply) {
  const countries: Record<number, unknown> = {};
  for (const row of reply.countries) countries[row.countryId] = [row.tag, row.total > 0, row.counts.map(v => v.ordinal)];
  return JSON.stringify([reply.missing, reply.types, reply.warnings, countries, reply.countries.map(row => row.countryId)]);
}

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

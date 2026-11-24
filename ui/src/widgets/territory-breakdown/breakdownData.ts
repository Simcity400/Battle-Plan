import type { BreakdownLeafItem as SunburstItem, BreakdownLeafKey as SunburstKey } from '../../families/breakdown/tree';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { NO_ROW } from '../../dashboard/follow/liveCells.ts';
import type { TerritoryMeasure } from '../../dashboard/territoryMeasure';
import type { CountryGroupReply, FactionGroup } from '../../generated/dashboard';
import type { TerritoryBreakdownReply } from './types';

export type RegionalRule = 'controlled' | 'owned';

export type Affiliation = { faction?: FactionGroup; unknown: boolean };

export function affiliation(groups: CountryGroupReply, hour: number): (tag: string) => Affiliation {
  const covers = (spans: readonly { firstHour: number; lastHour: number }[]) => spans.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const unread = covers(groups.unobserved.factions);
  const active = groups.factions.filter(faction => faction.firstHour <= hour && hour <= faction.lastHour);
  const incomplete = unread || active.some(faction => covers(faction.incomplete));
  const members = new Map<string, FactionGroup[]>();
  for (const faction of active) for (const member of faction.members) {
    if (member.firstHour > hour || hour > member.lastHour) continue;
    const list = members.get(member.tag) ?? [];
    list.push(faction);
    members.set(member.tag, list);
  }
  return tag => {
    const found = members.get(tag) ?? [];
    return { faction: found.length === 1 ? found[0] : undefined, unknown: incomplete || found.length > 1 };
  };
}

export function affiliationEpochs(groups: CountryGroupReply): number[] {
  const hours = new Set<number>();
  const add = (span: { firstHour: number; lastHour: number }) => { hours.add(span.firstHour); hours.add(span.lastHour + 1); };
  for (const span of groups.unobserved.factions) add(span);
  for (const faction of groups.factions) {
    add(faction);
    for (const span of faction.incomplete) add(span);
    for (const member of faction.members) add(member);
  }
  return [...hours].sort((a, b) => a - b);
}

export function epochOf(epochs: readonly number[], hour: number): number {
  let low = 0, high = epochs.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (epochs[mid]! <= hour) low = mid + 1; else high = mid;
  }
  return low;
}

export interface Amount { value: number; exact: bigint; denominator: bigint }

const NOTHING: Amount = { value: 0, exact: 0n, denominator: 1n };
const whole = (value: number) => BigInt(Math.trunc(value));

export function rowAmount(values: LiveValues<number>, key: string, measure: TerritoryMeasure): Amount | null {
  const at = liveKey(key, 'value');
  const amount = values.get(at);
  if (amount === undefined) return values.missing(at) === NO_ROW ? NOTHING : null;
  if (measure !== 'states') return { value: amount, exact: whole(amount), denominator: 1n };
  const denominator = values.get(liveKey(key, 'denominator'));
  if (denominator === undefined || denominator <= 0) return null;
  return { value: amount / denominator, exact: whole(amount), denominator: whole(denominator) };
}

export interface BreakdownLooks {
  country: (tag: string) => NonNullable<SunburstKey['countryFlag']> & { name: string };
  faction: (faction: FactionGroup) => string;
  colour: (id: string) => string | undefined;
  selected: (tag: string | null) => boolean;
}

export function breakdownItems(reply: TerritoryBreakdownReply, values: LiveValues<number>, measure: TerritoryMeasure,
  factionOf: (tag: string) => Affiliation, looks: BreakdownLooks): SunburstItem[] {
  const regions = new Map(reply.regions.map(region => [region.id, region]));
  const items: SunburstItem[] = [];
  for (const row of reply.rows) {
    if (!looks.selected(row.tag)) continue;
    const amount = rowAmount(values, row.key, measure);
    if (amount && amount.exact === 0n) continue;
    const held = row.slot !== null;
    const member: Affiliation = row.tag !== null ? factionOf(row.tag) : { unknown: false };
    const country = row.tag !== null ? looks.country(row.tag) : null;
    const region = row.region === null ? null : regions.get(row.region);
    const path: SunburstKey[] = [
      { id: member.unknown ? 'faction-unknown' : member.faction ? `faction:${member.faction.id}` : held ? 'non-aligned' : 'unassigned-territory',
        label: member.unknown ? 'Faction not recorded' : member.faction ? looks.faction(member.faction) : held ? 'Non-aligned' : 'Unassigned territory',
        ...(member.faction && !member.unknown && looks.colour(member.faction.id) ? { colour: looks.colour(member.faction.id) } : {}) },
      { id: JSON.stringify(row.tag !== null ? ['country', row.tag] : held ? ['slot', row.slot] : ['unassigned']),
        label: country?.name ?? (held ? 'Country not named at this hour' : 'Unassigned territory'), ...(country ? { countryFlag: country } : {}) },
      { id: row.region === null ? 'region-unattributed' : String(row.region),
        label: region?.name ?? (row.region === null ? 'Unknown region' : `Region ${row.region}`) },
    ];
    items.push(amount ? { path, value: amount.value, exact: amount.exact, ...(amount.denominator !== 1n ? { exactDenominator: amount.denominator } : {}),
      unknownLevels: [member.unknown, false, false] } : { path, value: null, exact: null, unknownValue: true });
  }
  return items;
}

const pathKey = (item: SunburstItem) => item.path.map(key => key.id).join('\u0000');

export function withFinals(items: readonly SunburstItem[], ending: readonly SunburstItem[]): SunburstItem[] {
  const finals = new Map(ending.map(item => [pathKey(item), item]));
  const out: SunburstItem[] = items.map(item => {
    const end = finals.get(pathKey(item));
    finals.delete(pathKey(item));
    return end?.unknownValue ? { ...item, final: null, finalUnknown: true } : { ...item, final: end?.value ?? 0 };
  });
  for (const end of finals.values()) out.push(end.unknownValue ? { path: end.path, value: 0, exact: 0n, final: null, finalUnknown: true }
    : { path: end.path, value: 0, exact: 0n, final: end.value });
  return out;
}

const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const count = (value: unknown) => Number.isSafeInteger(value);

export function validateBreakdownReply(value: unknown): TerritoryBreakdownReply {
  if (!object(value) || typeof value.measure !== 'string' || typeof value.rule !== 'string'
    || !Array.isArray(value.regions) || !value.regions.every(region => object(region) && count(region.id) && text(region.name) && text(region.missing))
    || !Array.isArray(value.rows) || !value.rows.every(row => object(row) && typeof row.key === 'string' && text(row.tag)
      && (row.slot === null || count(row.slot)) && (row.region === null || count(row.region)))
    || !text(value.missing) || !text(value.areaReason) || !Array.isArray(value.warnings) || !value.warnings.every(item => typeof item === 'string'))
    throw new Error('The territory breakdown reply is malformed.');
  return value as unknown as TerritoryBreakdownReply;
}

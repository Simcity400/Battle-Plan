import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CountryGroupReply, FactionGroup, TerritoryBreakdownReply, TerritoryMeasure, TerritoryRule } from '../../generated/dashboard';
import type { SunburstItem, SunburstKey, SunburstTotal } from '../../charts/sunburst/Sunburst';
import { isOrderedSpans, runAt, stretchAt, stretchStarts } from '../../charts/series.ts';
import { fractionSum } from '../../charts/valueAxis.ts';
import { intervalIndexOf } from '../../charts/intervalIndex.ts';
import { object, opaqueTag, unsigned32, unsigned64, signed32, signed64 } from '../../dashboard/validation.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { territoryColumn, validateTerritoryReply } from '../territory/territoryData.ts';

const text = (value: unknown) => value === null || typeof value === 'string';
export function validateBreakdownReply(value: unknown): TerritoryBreakdownReply {
  if (!object(value) || typeof value.branchId !== 'string' || !Array.isArray(value.regions)
    || !value.regions.every(r => object(r) && unsigned32(r.id) && text(r.name) && text(r.missing))
    || new Set(value.regions.map(r => r.id)).size !== value.regions.length || !Array.isArray(value.series)
    || !value.series.every(s => object(s) && (s.tag === null || opaqueTag(s.tag)) && (s.region === null || unsigned32(s.region))
      && text(s.missing) && Array.isArray(s.runs) && s.runs.every((r, i) => object(r) && signed32(r.firstHour) && signed32(r.lastHour)
        && r.firstHour <= r.lastHour && (i === 0 || (s.runs as { lastHour: number }[])[i - 1].lastHour < r.firstHour)
        && typeof r.startsSegment === 'boolean' && Array.isArray(r.values) && r.values.length === 6
        && r.values.every((n, column) => n === null || signed64(n) && (column % 3 === 1 || BigInt(n) >= 0n))
        && Array.isArray(r.stateShares) && r.stateShares.length === 2 && r.stateShares.every(terms => terms === null
          || Array.isArray(terms) && terms.every((term, index) => object(term) && unsigned64(term.numerator)
            && unsigned32(term.denominator) && term.denominator > 0
            && (index === 0 || (terms[index - 1] as { denominator: number }).denominator < term.denominator)))))
    || !Array.isArray(value.observed) || value.observed.length !== 8 || !value.observed.every(isOrderedSpans)
    || new Set(value.series.map(s => JSON.stringify([s.tag, s.region, s.missing]))).size !== value.series.length)
    throw new Error('The territory breakdown reply is malformed.');
  validateTerritoryReply({ ...value, series: value.countries });
  return value as unknown as TerritoryBreakdownReply;
}

function affiliation(groups: CountryGroupReply, hour: number): (tag: string) => { faction?: FactionGroup; unknown: boolean } {
  const unread = !!runAt(groups.unobserved.factions, hour);
  const active = intervalIndexOf(groups.factions, () => groups.factions).at(hour);
  const incomplete = unread || active.some(f => !!runAt(f.incomplete, hour));
  const members = new Map<string, FactionGroup[]>();
  for (const faction of active) for (const member of intervalIndexOf(faction.members, () => faction.members).at(hour)) {
    const list = members.get(member.tag) ?? []; list.push(faction); members.set(member.tag, list);
  }
  return tag => { const found = members.get(tag) ?? []; return { faction: found.length === 1 ? found[0] : undefined, unknown: incomplete || found.length > 1 }; };
}

export interface BreakdownLooks {
  country: (tag: string, hour: number) => ReturnType<CountryLabel>;
  faction: (faction: FactionGroup, hour: number) => { name: string | null; reason: string | null };
  colour: (id: string) => string | undefined;
  selected: (tag: string | null, hour: number) => boolean;
}
/** Resolve the chosen column and its unbroken stretches once per reply. */
export function breakdownModel(data: TerritoryBreakdownReply, measure: TerritoryMeasure, rule: TerritoryRule) {
  const regions = new Map(data.regions.map(region => [region.id, region]));
  const countries = new Map(data.countries.map(country => [country.tag, country]));
  const identities = countryIdentityIndex(data.countries.map(country => ({ tag: country.tag, identities: country.identities ?? [] })));
  const column = territoryColumn(measure, rule);
  const regionRuns = new Map(data.series.map(series => [series, series.runs.map(run => {
    if (measure === 'states') {
      const terms = run.stateShares[rule === 'owned' ? 1 : 0];
      const exact = terms === null ? null : fractionSum(terms.map(term => [BigInt(term.numerator), term.denominator] as const));
      const value = terms === null ? null : terms.reduce((sum, term) => sum + Number(term.numerator) / term.denominator, 0);
      return { ...run, exact, value };
    }
    const raw = run.values[column - (rule === 'owned' ? 2 : 1)];
    return { ...run, exact: raw === null ? null : [BigInt(raw), 1n] as const, value: raw === null ? null : Number(raw) };
  })]));
  const knownRuns = new Map([...regionRuns].map(([series, runs]) => [series, runs.filter(run => run.value !== null)]));
  const starts = new Map([...knownRuns].map(([series, runs]) => [series, stretchStarts(runs)]));
  const countryRuns = new Map(data.countries.map(country => [country.tag, country.runs.filter(run => run.values[column] !== null)]));
  const countryStarts = new Map([...countryRuns].map(([tag, runs]) => [tag, stretchStarts(runs)]));
  return { data, regions, countries, identities, column, regionRuns, knownRuns, starts, countryRuns, countryStarts };
}

export function breakdownItems(model: ReturnType<typeof breakdownModel>, groups: CountryGroupReply, hour: number, end: number,
  looks: BreakdownLooks): { items: SunburstItem[] } {
  const { data, regions, countries, column, regionRuns, knownRuns, starts, countryRuns, countryStarts } = model;
  const items: SunburstItem[] = [];
  for (const [at, final] of [[hour, false], [end, true]] as const) {
    const factionOf = affiliation(groups, at);
    for (const series of data.series) {
      if (!looks.selected(series.tag, at)) continue;
      const run = runAt(regionRuns.get(series)!, at);
      // Complete country/world totals also prove an absent region contributes zero.
      // Missing whole readings do not establish that absence.
      const whole = series.tag !== null ? runAt(countries.get(series.tag)?.runs ?? [], at)?.values[column]
        : runAt(data.worldTotals, at)?.values[column];
      const absent = !run && whole === 0 && series.tag !== null && !runAt(countries.get(series.tag)?.segments ?? [], at);
      // Complete holder and geographic inputs prove unlisted anonymous parts absent.
      const present = !absent && (series.tag !== null || (run ? run.value !== 0 : !runAt(data.observed[column], at)));
      const exact = run ? run.exact : whole != null || !present ? [0n, 1n] as const : null;
      const value = run ? run.value : exact === null ? null : 0;
      const holderUnknown = series.tag === null && !!series.missing?.startsWith('Country holder');
      const member = series.tag !== null ? factionOf(series.tag) : { unknown: holderUnknown, faction: undefined };
      const faction = member.faction ? looks.faction(member.faction, at) : null;
      const country = series.tag !== null ? looks.country(series.tag, at) : { name: holderUnknown ? 'Country not recorded' : 'Unassigned territory', flag: undefined };
      const region = series.region === null ? null : regions.get(series.region);
      const path: SunburstKey[] = [
        { id: member.unknown ? 'faction-unknown' : member.faction ? `faction:${member.faction.id}` : series.tag !== null ? 'non-aligned' : 'unassigned-territory',
          label: member.unknown ? 'Faction not recorded' : faction ? faction.name ?? 'Unnamed faction' : series.tag !== null ? 'Non-aligned' : 'Unassigned territory',
          colour: member.faction && !member.unknown ? looks.colour(member.faction.id) : undefined },
        { id: JSON.stringify(series.tag !== null ? ['country', series.tag] : ['holder', holderUnknown ? 'unknown' : 'unassigned']), label: country.name,
          countryFlag: series.tag !== null ? country : undefined,
          stretch: series.tag !== null ? stretchAt(countryRuns.get(series.tag) ?? [], countryStarts.get(series.tag) ?? [], at) : null },
        { id: series.region === null ? 'region-unattributed' : String(series.region), label: region?.name ?? (series.region === null ? 'Unknown region' : `Region ${series.region}`),
          stretch: stretchAt(knownRuns.get(series)!, starts.get(series)!, at) },
      ];
      const unknownLevels = [present && member.unknown, present && holderUnknown, false];
      items.push({ path, value: final ? 0 : value, exact: final ? 0n : exact?.[0] ?? null,
        exactDenominator: final ? undefined : exact?.[1],
        unknownValue: !final && present && exact === null, unknownLevels: final ? undefined : unknownLevels,
        final: final ? value : 0,
        finalUnknown: final && present && exact === null, finalUnknownLevels: final ? unknownLevels : undefined,
        present: !final && present });
    }
  }
  return { items };
}

export function breakdownWhole(data: TerritoryBreakdownReply, hour: number, end: number,
  measure: TerritoryMeasure, rule: TerritoryRule): SunburstTotal {
  const column = territoryColumn(measure, rule);
  const value = runAt(data.worldTotals, hour)?.values[column] ?? null;
  const final = runAt(data.worldTotals, end)?.values[column] ?? null;
  return { value, exact: value === null ? null : BigInt(value), unknownValue: value === null,
    final, finalUnknown: final === null, note: value === null ? 'World territory not recorded at this hour' : undefined };
}

/** Chosen-country totals stay independent of unresolved faction/region attribution. */
export function breakdownSelectedWhole(data: TerritoryBreakdownReply, hour: number, end: number,
  measure: TerritoryMeasure, rule: TerritoryRule, selected: BreakdownLooks['selected'], identities: CountryIdentityIndex): SunburstTotal {
  const column = territoryColumn(measure, rule);
  const total = (at: number) => {
    if (!runAt(data.observed[column], at)) return null;
    let exact = 0n;
    for (const country of data.countries) {
      const value = runAt(country.runs, at)?.values[column];
      if (identities.keyAt(country.tag, at) === null) {
        // A complete absent-country zero needs no holder attribution.
        if (value !== 0 || runAt(country.segments, at)) return null;
      } else if (selected(country.tag, at)) {
        if (value == null) return null;
        exact += BigInt(value);
      }
    }
    return exact;
  };
  const value = total(hour), final = total(end);
  return { value: value === null ? null : Number(value), exact: value, unknownValue: value === null,
    final: final === null ? null : Number(final), finalUnknown: final === null };
}

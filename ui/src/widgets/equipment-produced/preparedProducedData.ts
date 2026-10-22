import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { ProducedGrouping, ProducedKey, ProducedParams, ProducedType } from '../../generated/dashboard';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { domainName, variantNameAt } from '../../dashboard/equipment.ts';
import { formatAmount, formatRate } from '../../charts/valueAxis.ts';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import type { PreparedPrefixFields } from '../../dashboard/preparedPrefix';

export interface ProductionValue { key: string; amount: string | null; equipmentAmount: string | null }
export interface ProductionCountry {
  tag: string; slot: number; countryId: number; segment: string; total: string | null; equipmentTotal: string | null;
  readHour: number; observation: number | null; values: ProductionValue[];
}
/** This mirrors the prepared Rust selection, separately from delta-coded histories. */
export interface ProductionSelection {
  schema: number; branchId: string; firstHour: number; lastHour: number; known: boolean;
  keys: ProducedKey[]; types: ProducedType[]; totalCountries: number;
  offset: string; valueMin: string; valueMax: string;
  countries: ProductionCountry[]; warnings: string[];
  typeTotals?: [string, string][] | null;
  /** The hour every running total at this page began counting, whichever countries are selected. */
  observation: number | null;
  /** Before preparation completes, the prepared prefix the page answers. */
  coverage?: PreparedPrefixFields['coverage']; pending?: PreparedPrefixFields['pending']; approximate?: PreparedPrefixFields['approximate'];
}
export interface ProductionSelectionRequest { hour: number; selection: ProducedParams; offset: string; limit: number; ranked: boolean }
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const quantity = (value: unknown): value is string | null => value === null || typeof value === 'string' && /^-?(0|[1-9]\d*)$/.test(value);
const text = (value: unknown) => value === null || typeof value === 'string';

/** The hour a page answers for `request`: the requested hour, or, before
 * preparation reaches it, the prepared prefix's last hour. */
export const answeredHour = (page: ProductionSelection, request: ProductionSelectionRequest) => {
  const prefix = preparedPrefix(page);
  return prefix?.pending.length ? Math.min(request.hour, prefix.coverage.lastHour) : request.hour;
};

export function validateProductionSelection(value: unknown, request: ProductionSelectionRequest): ProductionSelection {
  const page = value as ProductionSelection;
  const hour = page != null && typeof page === 'object' ? answeredHour(page, request) : request.hour;
  const ok = page != null && typeof page === 'object' && page.schema === 1 && page.branchId === request.selection.branchId
    && integer(page.firstHour) && integer(page.lastHour) && page.firstHour <= hour && hour <= page.lastHour
    && typeof page.known === 'boolean' && integer(page.totalCountries) && page.totalCountries >= 0
    && page.offset === request.offset && quantity(page.valueMin) && page.valueMin !== null
    && quantity(page.valueMax) && page.valueMax !== null && BigInt(page.valueMin) <= 0n && BigInt(page.valueMax) >= 0n
    && Array.isArray(page.keys) && page.keys.every(key => typeof key.key === 'string' && text(key.name) && text(key.typeKey)
      && Array.isArray(key.names) && key.names.every((name, index) => integer(name.firstHour) && text(name.name)
        && typeof name.known === 'boolean' && text(name.design) && (index === 0 || key.names[index - 1].firstHour < name.firstHour)))
    && new Set(page.keys.map(key => key.key)).size === page.keys.length
    && Array.isArray(page.types) && page.types.every(type => typeof type.key === 'string' && text(type.name))
    && Array.isArray(page.countries) && page.countries.length <= request.limit && page.countries.every(country => typeof country.tag === 'string'
      && integer(country.slot) && country.slot >= 0 && integer(country.countryId) && country.countryId >= 0
      && typeof country.segment === 'string' && /^(0|[1-9]\d*)$/.test(country.segment)
      && integer(country.readHour) && country.readHour <= hour && (country.observation === null || integer(country.observation))
      && quantity(country.total) && quantity(country.equipmentTotal) && Array.isArray(country.values)
      && country.values.every(item => typeof item.key === 'string' && page.keys.some(key => key.key === item.key)
        && quantity(item.amount) && quantity(item.equipmentAmount))
      && new Set(country.values.map(item => item.key)).size === country.values.length)
    && new Set(page.countries.map(country => country.tag)).size === page.countries.length
    && (page.typeTotals == null || Array.isArray(page.typeTotals) && page.typeTotals.every(pair => Array.isArray(pair) && pair.length === 2
      && page.keys.some(key => key.key === pair[0]) && quantity(pair[1]) && pair[1] !== null))
    && (page.observation === null || integer(page.observation))
    && Array.isArray(page.warnings) && page.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The prepared equipment produced page is malformed.');
  return page;
}

/** Same positive-total choice and key-order tie break as the history model. */
export function mostDeliveredPrefix(page: ProductionSelection | undefined): string | null {
  let best: string | null = null, maximum = 0;
  for (const [key, raw] of page?.typeTotals ?? []) {
    const total = Number(raw);
    if (total > maximum) { best = key; maximum = total; }
  }
  return best;
}

export function productionKeyLabel(key: ProducedKey, grouping: ProducedGrouping, hour: number): string {
  return grouping === 'domain' ? domainName(key.key) : grouping === 'variant' ? variantNameAt(key.names, hour, `Variant ${key.key}`)
    : contentTitle(key.name, key.key, 'Equipment type name unavailable');
}
export const exactProductionAmount = (raw: string | null, decimals = 0): string => raw === null ? 'Not known' : formatAmount(raw, decimals, decimals);
/** The same units/IC display rule as the history view, from exact endpoint operands. */
export const formatProductionAmount = (raw: string | null, decimals = 0): string => raw === null ? 'Not known'
  : decimals === 0 ? formatAmount(raw, 0, 0) : formatRate(raw, decimals);

export interface ProductionBand { id: string; label: string; color: string; keys: readonly string[] }
/** A display fold is sized by the palette; all source keys remain in the tooltip. */
export function productionBands(page: ProductionSelection, grouping: ProducedGrouping, hour: number, palette: readonly string[], muted: string, country?: ProductionCountry): ProductionBand[] {
  let keys = page.keys;
  if (grouping === 'variant' && country) {
    const amounts = new Map(country.values.map(value => [value.key, value.amount]));
    const magnitude = (key: ProducedKey) => { const value = BigInt(amounts.get(key.key) ?? '0'); return value < 0n ? -value : value; };
    keys = keys.filter(key => amounts.has(key.key)).map((key, order) => ({ key, order, amount: magnitude(key) }))
      .sort((left, right) => left.amount === right.amount ? left.order - right.order : left.amount > right.amount ? -1 : 1).map(item => item.key);
  }
  const colored = keys.length > palette.length ? Math.max(0, palette.length - 1) : keys.length;
  const bands = keys.slice(0, colored).map((key, index) => ({ id: key.key, label: productionKeyLabel(key, grouping, hour), color: palette[index] ?? muted, keys: [key.key] }));
  if (colored < keys.length) bands.push({ id: '', label: 'Other', color: muted, keys: keys.slice(colored).map(key => key.key) });
  return bands;
}
/** Sum exact signed operands before the sole plotting conversion. */
export function productionBandAmount(country: ProductionCountry, band: ProductionBand, equipment = false): string | null {
  const values = new Map(country.values.map(value => [value.key, value]));
  let sum = 0n;
  for (const key of band.keys) {
    const item = values.get(key);
    const value = item ? equipment ? item.equipmentAmount : item.amount : '0';
    if (value === null) return null;
    sum += BigInt(value);
  }
  return sum.toString();
}
export function productionRaceEntries(page: ProductionSelection, bands: (country: ProductionCountry) => readonly ProductionBand[], split: boolean, hour = page.firstHour, decimals = 0): CountryRaceEntry[] {
  return page.countries.filter(country => country.total !== null).map(country => ({ tag: country.tag,
    entityKey: `${country.slot}:${country.countryId}`, hour, value: Number(country.total) / 10 ** decimals,
    label: formatProductionAmount(country.total, decimals), stretch: `${country.slot}:${country.countryId}:${country.segment}:${country.observation}`,
    ...(split ? { parts: (() => {
      const values = bands(country).map(band => ({ band, raw: productionBandAmount(country, band) }));
      const shown = values.filter((value): value is { band: ProductionBand; raw: string } => value.raw !== null);
      const parts: NonNullable<CountryRaceEntry['parts']>[number][] = shown.map(({ band, raw }) => ({ id: band.id, color: band.color, value: Number(raw) / 10 ** decimals }));
      if (shown.length < values.length) {
        const known = shown.reduce((sum, value) => sum + BigInt(value.raw), 0n);
        parts.push({ id: 'unknown', value: 0, hatched: Number(BigInt(country.total!) - known) / 10 ** decimals });
      }
      return parts;
    })() } : {}) }));
}

import { clipToSpans, isOrderedSpans } from '../charts/series.ts';
import type { Span } from '../charts/series';
import type { CountryIdentityHistory } from '../generated/dashboard';
import type { CountrySelection } from './model';
import { countryIdentityIndex } from './countryIdentity.ts';
import { contentKey, setToken } from './options.ts';
import { object, signed32 } from './validation.ts';

export interface DatedCountrySelection { campaign: string; branch: string; tag: string; originOffset: number; extent?: [number, number] }
declare const BOUND: unique symbol;
export type BoundCountry = DatedCountrySelection & { readonly [BOUND]: 'gated' };
export const boundCountryToken = (selection: BoundCountry) => setToken(JSON.stringify(selection));

export function readDatedCountry(token: unknown, campaign: string, branch: string, countries?: CountrySelection): DatedCountrySelection | null {
  if (typeof token !== 'string' || !token) return null;
  try {
    const value: unknown = JSON.parse(contentKey(token));
    return object(value) && value.campaign === campaign && value.branch === branch && typeof value.tag === 'string' && value.tag.length > 0
      && typeof value.originOffset === 'number' && Number.isSafeInteger(value.originOffset) && value.originOffset >= 0
      && (value.extent === undefined || Array.isArray(value.extent) && value.extent.length === 2
        && value.extent.every(Number.isSafeInteger) && value.extent[0] <= value.extent[1])
      && (!countries || countries.kind === 'tags' && countries.tags.length === 1 && countries.tags[0] === value.tag)
      ? value as unknown as DatedCountrySelection : null;
  } catch { return null; }
}

export function datedCountrySpans(histories: readonly CountryIdentityHistory[], selection: DatedCountrySelection,
  firstHour: number, lastHour: number, membership?: ReadonlyMap<string, readonly Span[]> | null): ReadonlyMap<string, readonly Span[]> | null {
  const originHour = firstHour + selection.originOffset;
  if (!signed32(originHour) || originHour < firstHour || originHour > lastHour) return null;
  const index = countryIdentityIndex(histories), key = index.keyAt(selection.tag, originHour);
  if (key === null) return null;
  const aliases = index.aliasRuns(new Set([key]), firstHour, lastHour);
  if (!isOrderedSpans(aliases)) return null;
  const spans = new Map<string, Span[]>();
  for (const alias of aliases) {
    const coverage = membership ? clipToSpans([alias], membership.get(alias.tag) ?? []) : [alias];
    if (!coverage.length) continue;
    const previous = spans.get(alias.tag);
    if (previous) previous.push(...coverage); else spans.set(alias.tag, coverage);
  }
  return spans;
}

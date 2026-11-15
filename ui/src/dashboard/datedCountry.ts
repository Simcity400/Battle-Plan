import { clipToSpans, isOrderedSpans } from '../charts/series.ts';
import type { Span } from '../charts/series';
import type { CountryIdentityHistory } from '../generated/dashboard';
import type { CountrySelection } from './model';
import { countryIdentityIndex } from './countryIdentity.ts';
import { contentKey, setToken } from './options.ts';
import { object, signed32 } from './validation.ts';

export interface DatedCountrySelection { campaign: string; branch: string; tag: string; originOffset: number; extent?: [number, number] }
export const datedCountryToken = (selection: DatedCountrySelection) => setToken(JSON.stringify(selection));

/** Store the original choice, never a later alias or a runtime handle. */
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

/** The country a choice that saves one shows: the one saved, else (its automatic choice) the widget's own country `tag` as
 * it was at `origin`, the hour the choice was first shown. Null when neither is known. */
export function chosenDatedCountry(token: unknown, campaign: string, branch: string, countries: CountrySelection, tag: string | undefined,
  firstHour: number, lastHour: number, origin: number): DatedCountrySelection | null {
  if (typeof token === 'string' && token) return readDatedCountry(token, campaign, branch, countries);
  if (!tag || !Number.isSafeInteger(origin) || origin < firstHour || origin > lastHour) return null;
  return { campaign, branch, tag, originOffset: origin - firstHour };
}

/** Strict native coverage: missing origins and conflicting aliases never fall back to a tag segment. */
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

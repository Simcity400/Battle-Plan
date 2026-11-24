import type { CountrySpan, FacetFilter, HourSpan, MembershipRun, UnobservedSpan } from '../../generated/dashboard';
import type { BoundHour, BoundWindow } from '../../dashboard/regionTime';

export type CountryScopeFilter<H extends number = BoundHour> =
  | { readonly kind: 'countries'; readonly tags: readonly string[] }
  | { readonly kind: 'membership'; readonly membership: readonly (Omit<CountrySpan, 'firstHour' | 'lastHour'> & { readonly firstHour: H; readonly lastHour: H })[]; readonly unobserved: readonly (Omit<UnobservedSpan, 'firstHour' | 'lastHour'> & { readonly firstHour: H; readonly lastHour: H })[] };

export type LogFacet<H extends number = BoundHour> = Omit<FacetFilter, 'membership' | 'unobserved'> & {
  membership?: readonly (Omit<CountrySpan, 'firstHour' | 'lastHour'> & { firstHour: H; lastHour: H })[];
  unobserved?: readonly (Omit<UnobservedSpan, 'firstHour' | 'lastHour'> & { firstHour: H; lastHour: H })[];
};

export function boundMembershipScope(
  membership: readonly MembershipRun[],
  unobserved: readonly HourSpan[],
  bound: (firstHour: number, lastHour: number) => BoundWindow | null,
): CountryScopeFilter<BoundHour> {
  const runs: { country: string; firstHour: BoundHour; lastHour: BoundHour }[] = [];
  for (const run of membership) {
    const span = bound(run.firstHour, run.lastHour);
    if (span) runs.push({ country: run.tag, firstHour: span.firstHour, lastHour: span.lastHour });
  }
  const gaps: { firstHour: BoundHour; lastHour: BoundHour }[] = [];
  for (const gap of unobserved) {
    const span = bound(gap.firstHour, gap.lastHour);
    if (span) gaps.push({ firstHour: span.firstHour, lastHour: span.lastHour });
  }
  return { kind: 'membership', membership: runs, unobserved: gaps };
}

export function countryScopeFacet<H extends number = BoundHour>(
  name: string,
  scope: readonly string[] | CountryScopeFilter<H> | null,
): LogFacet<H> | null {
  if (!scope) return null;
  if (Array.isArray(scope)) {
    return scope.length > 0 ? { name, values: [], countries: [...scope] } : null;
  }
  const filter = scope as CountryScopeFilter<H>;
  if (filter.kind === 'countries') {
    return filter.tags.length > 0 ? { name, values: [], countries: [...filter.tags] } : null;
  }
  if (filter.membership.length === 0 && filter.unobserved.length === 0) return null;
  return {
    name,
    values: [],
    countries: [],
    membership: filter.membership.map(run => ({ country: run.country, firstHour: run.firstHour, lastHour: run.lastHour })),
    ...(filter.unobserved.length > 0 ? { unobserved: filter.unobserved.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour })) } : {}),
  };
}

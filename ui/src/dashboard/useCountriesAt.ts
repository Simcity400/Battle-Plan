import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { FactSpec } from './countryFacts';
import { heldFacts } from './countryTimeline.ts';

export const STYLES: FactSpec<CountryStyle> = {
  product: 'country-styles', perRequest: Infinity, shared: true,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.map(style => [style.tag, style] as const)),
  held: heldFacts(({ tag, name, color }) => ({ tag, name, color })),
};

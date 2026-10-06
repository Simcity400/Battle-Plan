import { runAt } from '../charts/series.ts';
import type { CountryIdentityRun } from '../generated/dashboard';

export const countryIdentityKey = (identity: Pick<CountryIdentityRun, 'slot' | 'countryId'>): string => JSON.stringify([identity.slot, identity.countryId]);
/** Manual recorded-tag picks follow the same native country through a rename.
 * A reused tag's distinct identity remains distinct; tags themselves stay opaque. */
export function countryAliasesAt(histories: readonly { tag: string; identities: readonly CountryIdentityRun[] }[], chosen: readonly string[], hour: number): string[] {
  const tags = new Set(chosen);
  const identities = new Set<string>();
  for (const history of histories) if (tags.has(history.tag)) {
    for (const identity of history.identities) identities.add(countryIdentityKey(identity));
  }
  return histories.flatMap(history => {
    const identity = runAt(history.identities, hour);
    return identity && identities.has(countryIdentityKey(identity)) ? [history.tag] : [];
  });
}

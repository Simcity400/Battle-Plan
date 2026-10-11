import { useMemo } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { FAILED_FACT } from '../../dashboard/countryFacts';
import type { FactSpec } from '../../dashboard/countryFacts';
import { artworkEntry, keepDrawableIcons } from '../../dashboard/artwork';
import type { Artwork, TechnologyArtworkReply } from '../../generated/dashboard';

type ImageFact = Artwork & { warnings: string[] };
const SPEC: FactSpec<ImageFact> = {
  product: 'technology-artwork', perRequest: 8,
  requestOrder: 'wanted',
  params: (_branch, _hour, tokens) => ({ tokens }),
  extract: (value, params) => {
    const reply = value as TechnologyArtworkReply;
    const wanted = new Set((params as { tokens: string[] }).tokens);
    if (!reply || !Array.isArray(reply.items) || !Array.isArray(reply.warnings) || !reply.warnings.every(item => typeof item === 'string')
      || !reply.items.every(artworkEntry) || reply.items.length !== wanted.size
      || new Set(reply.items.map(item => item.token)).size !== wanted.size || reply.items.some(item => !wanted.has(item.token))) {
      throw new Error('Technology artwork does not match this request.');
    }
    keepDrawableIcons(reply.items);
    return new Map(reply.items.map(item => [item.token, { ...item, warnings: reply.warnings }]));
  },
};

export function useTechnologyArtwork(client: DashboardClient, track: BoundTrack | undefined, tokens: readonly string[], active: boolean) {
  const wanted = useMemo(() => tokens.map(tag => ({ tag, hour: 0 })), [tokens]);
  const read = useCountryFactsRead(client, track, wanted, active, SPEC, 1);
  return useMemo(() => {
    const artwork = new Map<string, Artwork>(), warnings = new Set<string>();
    let failed = 0, missing = 0;
    for (const value of read.values()) {
      if (value === FAILED_FACT) failed++;
      else if (value === null) missing++;
      else {
        artwork.set(value.token, value);
        for (const warning of value.warnings) warnings.add(warning);
      }
    }
    if (failed) warnings.add(`Technology artwork request failed for ${failed} ${failed === 1 ? 'technology' : 'technologies'}.`);
    if (missing) warnings.add(`Technology artwork was not returned for ${missing} ${missing === 1 ? 'technology' : 'technologies'}.`);
    return { artwork, warnings: [...warnings] };
  }, [read]);
}

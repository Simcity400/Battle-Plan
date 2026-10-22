import { useMemo } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { FAILED_FACT } from '../../dashboard/countryFacts';
import type { FactSpec } from '../../dashboard/countryFacts';
import { artworkEntry, keepDrawableIcons } from '../../dashboard/artwork';
import type { Artwork, TechnologyArtworkParams, TechnologyArtworkReply } from '../../generated/dashboard';

type ImageFact = Artwork & { warnings: string[] };

/** JSON keeps quoted tokens lossless; repeated definitions have distinct template indices. */
function artworkTag(token: string, recorded: readonly (number | null)[] | undefined): string {
  const uses = [...new Set(recorded ?? [])].sort((a, b) => (a ?? 0) - (b ?? 0));
  return JSON.stringify(uses.some(index => index != null) ? [token, uses] : [token]);
}

function artworkRequest(tag: string): [string, (number | null)[]] {
  const [token, uses] = JSON.parse(tag) as [string, (number | null)[]?];
  return [token, uses ?? []];
}

const SPEC: FactSpec<ImageFact> = {
  product: 'technology-artwork', perRequest: 8,
  requestOrder: 'wanted',
  params: (_branch, _hour, tags): TechnologyArtworkParams => {
    const requests = tags.map(artworkRequest);
    const tokens = requests.map(([token]) => token);
    const recorded = Object.fromEntries(requests.filter(([, indices]) => indices.length));
    return Object.keys(recorded).length ? { tokens, recorded } : { tokens };
  },
  extract: (value, params) => {
    const reply = value as TechnologyArtworkReply;
    const { tokens, recorded } = params as TechnologyArtworkParams;
    const wanted = new Set(tokens);
    if (!reply || !Array.isArray(reply.items) || !Array.isArray(reply.warnings) || !reply.warnings.every(item => typeof item === 'string')
      || !reply.items.every(artworkEntry) || reply.items.length !== wanted.size
      || new Set(reply.items.map(item => item.token)).size !== wanted.size || reply.items.some(item => !wanted.has(item.token))) {
      throw new Error('Technology artwork does not match this request.');
    }
    keepDrawableIcons(reply.items);
    return new Map(reply.items.map(item => [artworkTag(item.token, recorded?.[item.token]), { ...item, warnings: reply.warnings }]));
  },
};

/** Findings belong to the recorded definitions, not merely their shared technology token. */
export function useTechnologyArtwork(client: DashboardClient, track: BoundTrack | undefined, tokens: readonly string[], active: boolean,
  recorded?: ReadonlyMap<string, readonly (number | null)[]>) {
  const wanted = useMemo(() => tokens.map(token => ({ tag: artworkTag(token, recorded?.get(token)), hour: 0 })), [tokens, recorded]);
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

import type { Failure } from '../../dashboard/queryError';
import { useMemo } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { nameKey, useCountryFactsRead } from '../../dashboard/countryNamesAt.ts';
import { isFailedFact } from '../../dashboard/countryFacts.ts';
import type { FactSpec } from '../../dashboard/countryFacts';
import { artworkEntry, keepDrawableIcons } from '../../dashboard/artwork';
import type { Artwork, TechnologyArtworkParams, TechnologyArtworkReply } from '../../generated/dashboard';

type ImageFact = Artwork & { warnings: string[] };

function artworkTag(token: string, recorded: readonly (number | null)[] | undefined): string {
  const uses = [...new Set(recorded ?? [])].sort((a, b) => (a ?? 0) - (b ?? 0));
  return JSON.stringify(uses.some(index => index != null) ? [token, uses] : [token]);
}

function artworkRequest(tag: string): [string, (number | null)[]] {
  const [token, uses] = JSON.parse(tag) as [string, (number | null)[]?];
  return [token, uses ?? []];
}

const SPEC: FactSpec<ImageFact> = {
  product: 'technology-artwork', perRequest: 64,
  requestOrder: 'wanted', shared: true,
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

export function useTechnologyArtwork(client: DashboardClient, track: BoundTrack | undefined, tokens: readonly string[], active: boolean,
  recorded?: ReadonlyMap<string, readonly (number | null)[]>) {
  const wanted = useMemo(() => tokens.map(token => ({ tag: artworkTag(token, recorded?.get(token)), hour: 0 })), [tokens, recorded]);
  const read = useCountryFactsRead(client, track, wanted, active, SPEC, 1);
  return useMemo(() => {
    const artwork = new Map<string, Artwork>(), failures = new Map<string, Failure>(), warnings = new Set<string>();
    let missing = 0;
    for (const { hour, tag } of wanted) {
      const value = read.get(nameKey(hour, tag));
      if (value === undefined) continue;
      if (isFailedFact(value)) { failures.set(artworkRequest(tag)[0], value); warnings.add(value.message); }
      else if (value === null) missing++;
      else {
        artwork.set(value.token, value);
        for (const warning of value.warnings) warnings.add(warning);
      }
    }
    if (missing) warnings.add(`Technology artwork was not returned for ${missing} ${missing === 1 ? 'technology' : 'technologies'}.`);
    return { artwork, failures, warnings: [...warnings] };
  }, [read, wanted]);
}

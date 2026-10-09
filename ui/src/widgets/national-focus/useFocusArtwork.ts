import { useMemo } from 'react';
import type { DatedFocusArtwork, FocusArtworkParams } from '../../generated/dashboard';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { FAILED_FACT } from '../../dashboard/countryFacts';
import type { FactSpec, NameAt } from '../../dashboard/countryFacts';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { validateFocusArtworkReply } from './focusData';

type FocusArtworkFact = DatedFocusArtwork & { warnings: readonly string[] };

/** Focus tokens reuse the dated fact store's client ownership and bounded queue. */
export function useFocusArtwork(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  wanted: readonly NameAt[], active: boolean) {
  const spec = useMemo<FactSpec<FocusArtworkFact>>(() => ({
    product: 'focus-artwork', identity: country, perRequest: 32,
    params: (branchId, hour, tokens) => ({ branchId, hour, tag: country, tokens }),
    extract: (data, params) => {
      const reply = validateFocusArtworkReply(data, params as FocusArtworkParams);
      return new Map(reply.items.map(item => [item.artwork.token, { ...item, warnings: reply.warnings }]));
    },
  }), [country]);
  const read = useCountryFactsRead(client, track, wanted, active && !!country, spec,
    Math.max(1, new Set(wanted.map(item => item.hour)).size));
  const warnings = useMemo(() => {
    const warnings = new Set<string>();
    for (const item of read.values()) if (item && item !== FAILED_FACT) {
      for (const warning of item.warnings) warnings.add(warning);
    }
    return [...warnings];
  }, [read]);
  return { read, warnings };
}

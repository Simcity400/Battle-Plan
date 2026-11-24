import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { Failure } from '../../dashboard/queryError.ts';
import { useMemo } from 'react';
import type { DatedFocusArtwork, FocusArtworkParams, FocusDatabase } from '../../generated/dashboard';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { isFailedFact } from '../../dashboard/countryFacts.ts';
import type { FactSpec, NameAt } from '../../dashboard/countryFacts';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { validateFocusArtworkReply } from './focusData';

type FocusArtworkFact = DatedFocusArtwork & { warnings: readonly string[]; titlePending?: boolean; titleReason?: string; titleFailure?: Failure; imagePending?: boolean; imageReason?: string; imageFailure?: Failure; descriptionFailure?: Failure };
type FocusFactsMode = 'titles' | 'images' | 'details';

function useFocusFacts(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  database: FocusDatabase, wanted: readonly NameAt[], active: boolean, mode: FocusFactsMode) {
  const spec = useMemo<FactSpec<FocusArtworkFact>>(() => ({
    product: 'focus-artwork', identity: JSON.stringify([country, database, mode]), perRequest: mode === 'images' ? 64 : 256,
    requestOrder: 'wanted', shared: true,
    params: (branchId, hour, tokens) => ({ branchId, hour, country: country === undefined ? undefined : countryFromKey(country), database, tokens,
      textOnly: mode !== 'images', includeDescriptions: mode === 'details',
      ...(mode === 'images' ? { iconsOnly: true } : {}) }),
    extract: (data, params) => {
      const reply = validateFocusArtworkReply(data, params as FocusArtworkParams);
      return new Map(reply.items.map(item => [item.artwork.token, { ...item, warnings: reply.warnings }]));
    },
  }), [country, database, mode]);
  const read = useCountryFactsRead(client, track, wanted, active && !!country, spec,
    Math.max(1, new Set(wanted.map(item => item.hour)).size));
  const warnings = useMemo(() => {
    const warnings = new Set<string>();
    for (const item of read.values()) {
      if (isFailedFact(item)) warnings.add(item.message);
      else if (item) for (const warning of item.warnings) warnings.add(warning);
    }
    return [...warnings];
  }, [read]);
  return { read, warnings };
}

export function useFocusText(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  wanted: readonly NameAt[], active: boolean) {
  return useFocusFacts(client, track, country, 'national', wanted, active, 'titles');
}

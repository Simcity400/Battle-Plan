import { useMemo } from 'react';
import type { Artwork, DatedFocusArtwork, FocusArtworkParams, FocusDatabase } from '../../generated/dashboard';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { FAILED_FACT, nameKey } from '../../dashboard/countryFacts';
import type { FactSpec, NameAt } from '../../dashboard/countryFacts';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { validateFocusArtworkReply } from './focusData';

type FocusArtworkFact = DatedFocusArtwork & { warnings: readonly string[]; titlePending?: boolean; titleReason?: string; imagePending?: boolean; imageReason?: string };
type FocusFactsMode = 'titles' | 'images' | 'details';
const NO_DETAILS: readonly NameAt[] = [];
const NO_STATIC_TEXT: readonly Artwork[] = [];
const NO_WARNINGS: readonly string[] = [];

/** Focus tokens reuse the dated fact store's client ownership and bounded queue. Each
 * database keeps its own store, so a national and a continuous focus of one id never share. */
function useFocusFacts(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  database: FocusDatabase, wanted: readonly NameAt[], active: boolean, mode: FocusFactsMode) {
  const spec = useMemo<FactSpec<FocusArtworkFact>>(() => ({
    product: 'focus-artwork', identity: JSON.stringify([country, database, mode]), perRequest: mode === 'images' ? 8 : 256,
    requestOrder: 'wanted',
    params: (branchId, hour, tokens) => ({ branchId, hour, tag: country, database, tokens,
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
    for (const item of read.values()) if (item && item !== FAILED_FACT) {
      for (const warning of item.warnings) warnings.add(warning);
    }
    return [...warnings];
  }, [read]);
  return { read, warnings };
}

/** Dated national focus text resolves independently; icons arrive in small batches. */
export function useFocusText(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  wanted: readonly NameAt[], active: boolean) {
  return useFocusFacts(client, track, country, 'national', wanted, active, 'titles');
}

function useDatedFocusArtwork(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  database: FocusDatabase, wanted: readonly NameAt[], active: boolean, detailsWanted: readonly NameAt[],
  staticText: readonly Artwork[]) {
  // The current validated history reply proves these names are country-independent.
  const staticTitles = useMemo(() => new Map(staticText.filter(item => item.name !== null).map(item => [item.token, item])), [staticText]);
  const titleWanted = useMemo(() => wanted.filter(item => !staticTitles.has(item.tag)), [wanted, staticTitles]);
  const text = useFocusFacts(client, track, country, database, titleWanted, active, 'titles');
  const images = useFocusFacts(client, track, country, database, wanted, active, 'images');
  const details = useFocusFacts(client, track, country, database, detailsWanted, active, 'details');
  const read = useMemo(() => {
    const merged = new Map(text.read);
    for (const item of wanted) {
      const key = nameKey(item.hour, item.tag), image = images.read.get(key), title = text.read.get(key), description = details.read.get(key);
      const known = staticTitles.get(item.tag);
      const titleArt = known ? { artwork: known, reason: null, warnings: NO_WARNINGS } : title && title !== FAILED_FACT ? title : null;
      const imageArt = image && image !== FAILED_FACT ? image : null;
      const detailArt = description && description !== FAILED_FACT ? description : null;
      const base = titleArt ?? imageArt ?? detailArt;
      if (base) merged.set(key, {
        ...base, artwork: { ...base.artwork, name: titleArt?.artwork.name ?? null,
          description: detailArt?.artwork.description ?? base.artwork.description, icon: imageArt?.artwork.icon ?? null },
        // Images and descriptions cannot settle the independent title read.
        titlePending: !known && title === undefined,
        titleReason: known ? undefined : title === FAILED_FACT ? 'Focus name request failed.'
          : title === null ? 'Focus name was not recorded at this hour.' : title?.reason ?? undefined,
        reason: null,
        imagePending: image === undefined,
        imageReason: image === FAILED_FACT ? 'Game artwork request failed.'
          : image === null ? 'Game artwork was not recorded at this hour.'
          : image?.reason ?? undefined,
      });
    }
    return merged;
  }, [text.read, images.read, details.read, wanted, staticTitles]);
  return { read, details: details.read, warnings: useMemo(() => [...new Set([...text.warnings, ...images.warnings, ...details.warnings])],
    [text.warnings, images.warnings, details.warnings]) };
}

/** National focuses: tree entries, completions and the current focus. */
export function useFocusArtwork(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  wanted: readonly NameAt[], active: boolean, detailsWanted: readonly NameAt[] = NO_DETAILS,
  staticText: readonly Artwork[] = NO_STATIC_TEXT) {
  return useDatedFocusArtwork(client, track, country, 'national', wanted, active, detailsWanted, staticText);
}

/** A country's active continuous focus, looked up in the continuous database. */
export function useContinuousFocusArtwork(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  wanted: readonly NameAt[], active: boolean, staticText: readonly Artwork[] = NO_STATIC_TEXT) {
  return useDatedFocusArtwork(client, track, country, 'continuous', wanted, active, NO_DETAILS, staticText);
}

import { countryFromKey } from '../../dashboard/countryKey.ts';
import { failureText, type Failure } from '../../dashboard/queryError.ts';
import { useMemo } from 'react';
import type { Artwork, DatedFocusArtwork, FocusArtworkParams, FocusDatabase } from '../../generated/dashboard';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { nameKey, isFailedFact } from '../../dashboard/countryFacts.ts';
import type { FactSpec, NameAt } from '../../dashboard/countryFacts';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { validateFocusArtworkReply } from './focusData';

type FocusArtworkFact = DatedFocusArtwork & { warnings: readonly string[]; titlePending?: boolean; titleReason?: string; titleFailure?: Failure; imagePending?: boolean; imageReason?: string; imageFailure?: Failure; descriptionFailure?: Failure };
type FocusFactsMode = 'titles' | 'images' | 'details';
const NO_DETAILS: readonly NameAt[] = [];
const NO_STATIC_TEXT: readonly Artwork[] = [];
const NO_WARNINGS: readonly string[] = [];

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

function useDatedFocusArtwork(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  database: FocusDatabase, wanted: readonly NameAt[], active: boolean, detailsWanted: readonly NameAt[],
  staticText: readonly Artwork[]) {
  const staticTitles = useMemo(() => new Map(staticText.filter(item => item.name !== null).map(item => [item.token, item])), [staticText]);
  const titleWanted = useMemo(() => wanted.filter(item => !staticTitles.has(item.tag)), [wanted, staticTitles]);
  const text = useFocusFacts(client, track, country, database, titleWanted, active, 'titles');
  const images = useFocusFacts(client, track, country, database, wanted, active, 'images');
  const details = useFocusFacts(client, track, country, database, detailsWanted, active, 'details');
  const reading = active && !!track && !!country;
  const read = useMemo(() => {
    const merged = new Map(text.read);
    for (const item of wanted) {
      const key = nameKey(item.hour, item.tag), image = images.read.get(key), title = text.read.get(key), description = details.read.get(key);
      const known = staticTitles.get(item.tag);
      const titleArt = known ? { artwork: known, reason: null, warnings: NO_WARNINGS } : title && !isFailedFact(title) ? title : null;
      const imageArt = image && !isFailedFact(image) ? image : null;
      const detailArt = description && !isFailedFact(description) ? description : null;
      const base = titleArt ?? imageArt ?? detailArt;
      if (base || [title, image, description].some(isFailedFact)) merged.set(key, {
        ...base, warnings: base?.warnings ?? NO_WARNINGS,
        artwork: { token: item.tag, baseDurationDays: null, ...base?.artwork, name: titleArt?.artwork.name ?? null,
          description: detailArt?.artwork.description ?? base?.artwork.description ?? null, icon: imageArt?.artwork.icon ?? null },
        titlePending: reading && !known && title === undefined,
        titleReason: known ? undefined : isFailedFact(title) ? failureText(title, 'Focus name request failed.')
          : title === null ? 'Focus name was not recorded at this hour.' : title?.reason ?? undefined,
        titleFailure: !known && isFailedFact(title) ? title : undefined,
        imageFailure: isFailedFact(image) ? image : undefined,
        descriptionFailure: isFailedFact(description) ? description : undefined,
        reason: null,
        imagePending: reading && image === undefined,
        imageReason: isFailedFact(image) ? failureText(image, 'Game artwork request failed.')
          : image === null ? 'Game artwork was not recorded at this hour.'
          : image?.reason ?? undefined,
      });
    }
    return merged;
  }, [text.read, images.read, details.read, wanted, staticTitles, reading]);
  return { read, details: details.read, warnings: useMemo(() => [...new Set([...text.warnings, ...images.warnings, ...details.warnings])],
    [text.warnings, images.warnings, details.warnings]) };
}

export function useFocusArtwork(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  wanted: readonly NameAt[], active: boolean, detailsWanted: readonly NameAt[] = NO_DETAILS,
  staticText: readonly Artwork[] = NO_STATIC_TEXT) {
  return useDatedFocusArtwork(client, track, country, 'national', wanted, active, detailsWanted, staticText);
}

export function useContinuousFocusArtwork(client: DashboardClient, track: BoundTrack | undefined, country: string | undefined,
  wanted: readonly NameAt[], active: boolean, staticText: readonly Artwork[] = NO_STATIC_TEXT) {
  return useDatedFocusArtwork(client, track, country, 'continuous', wanted, active, NO_DETAILS, staticText);
}

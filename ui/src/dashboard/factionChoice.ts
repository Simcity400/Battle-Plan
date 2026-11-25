import { useMemo } from 'react';
import type { FactionGroup } from '../generated/dashboard';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetBase, WidgetSpec } from './model';
import { setToken } from './options.ts';
import { selectedCountryKeys } from './countrySelection.ts';
import { countryFromKey } from './countryKey.ts';
import { factionNameAt as recordedNameAt } from './factionNames.ts';
import { useHeadProduct } from './headProduct.ts';
import { useGate } from './regionGate';
import { detailsStructure, validateFactionDetails } from './factionDetails.ts';
import { useCountryGroups } from './useCountryGroups';
import { useCampaignSettings } from './campaignSettings';

function factionNameAt(faction: FactionGroup, hour?: number, override?: string): string {
  return recordedNameAt(faction, hour, override).name ?? 'Faction name unavailable';
}

const NO_NAMES: Readonly<Record<string, string>> = Object.freeze({});

function useFactionDisplayNames(source: { metadata: { campaignKey: string } } | undefined): Readonly<Record<string, string>> {
  const styles = useCampaignSettings().campaigns[source?.metadata.campaignKey ?? '']?.factions;
  return useMemo(() => {
    if (!styles) return NO_NAMES;
    const named = Object.entries(styles).flatMap(([id, style]) => style.name ? [[id, style.name] as const] : []);
    return named.length ? Object.fromEntries(named) : NO_NAMES;
  }, [styles]);
}

export function useFactionDetails(runtime: WidgetRuntime, widget: WidgetBase, faction: string | null, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const params = useMemo(() => faction ? { faction, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } : null, [faction, viewpointTag]);
  return useHeadProduct(runtime, useGate(runtime, widget), 'faction-details', params,
    { active, validate: validateFactionDetails, structure: detailsStructure });
}

export function useFactionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, true);
  const names = useFactionDisplayNames(track?.source);
  return useMemo(() => ({ faction: (groups.data?.factions ?? []).map(faction => ({ value: setToken(faction.id),
    label: factionNameAt(faction, undefined, names[faction.id]) })) }), [groups.data, names]);
}

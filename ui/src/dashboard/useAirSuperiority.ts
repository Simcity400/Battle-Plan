import { useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { setToken } from './options';
import { useProduct } from './products';
import { useFactionColours } from './FactionColours';
import { useCountryLooks } from './countryLooks';
import { airSideName, factionNameChanges, factionNamer, validateAirChoices } from './airSuperiority';
import { airChoiceToken } from './airOptions';
import type { AirChoicesReply } from '../generated/dashboard';
import { productRequest } from './request.ts';
import { useRegion, useRegionHour } from './widgetTime';

export function useAirChoicesReply(runtime: WidgetRuntime, widget: WidgetSpec, active: boolean) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority-choices', params: { branchId: track.branch.id } }) : null;
  return useProduct<AirChoicesReply>(runtime.client, request, active, validateAirChoices, track?.source);
}

export function useAirChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const choices = useAirChoicesReply(runtime, widget, true);
  const factions = useFactionColours(runtime.client, track?.source);
  const hour = track?.branch.lastHour ?? 0;
  const tags = useMemo(() => [...new Set([...(choices.data?.countries ?? []), ...(choices.data?.sides.flatMap(side => [side.tag, side.leader].flatMap(tag => tag ? [tag] : [])) ?? [])])],
    [choices.data]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, true);
  const changes = useMemo(() => factionNameChanges(factions.factions), [factions.factions]);
  const shown = useRegionHour(runtime, useRegion(widget), true, changes);
  return useMemo(() => {
    const campaign = track?.source.metadata.campaignKey ?? '';
    const faction = factionNamer(factions.factions, factions.displayNames, shown);
    const country = (tag: string) => looks.name(tag, hour);
    const sides = (choices.data?.sides ?? []).map(side => ({ value: airChoiceToken(campaign, side.key), label: airSideName(side, country, faction) }));
    return {
      viewpoint: [...sides, ...(choices.data?.countries ?? []).map(tag => ({ value: setToken(`country:${tag}`), label: country(tag) }))], sides,
      theatres: (choices.data?.theatres ?? []).map(theatre => ({ value: airChoiceToken(campaign, theatre.id), label: theatre.name })),
    };
  }, [choices.data, looks, hour, shown, factions.factions, factions.displayNames, track?.source.metadata.campaignKey]);
}


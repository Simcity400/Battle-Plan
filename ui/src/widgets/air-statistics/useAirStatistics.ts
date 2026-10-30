import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { selectedTags } from '../../dashboard/countrySelection';
import { statisticName, validateAirStatistics } from './airStatisticsData';
import { airStatisticsChannel, airStatisticsRequest } from '../../dashboard/widgetRequests';
import type { AirStatisticsReply } from './airStatisticsData';
import { OPTIONS } from './definition';
import { choiceHour, theatreChoices, useAirTheatres } from '../air-missions/useAirTheatres';
import { usePlaybackHour } from '../../dashboard/playback.ts';

/** The country's prepared statistics history: one request per country and channel, looked up
 * locally at every playback hour. */
export function useAirStatistics(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const channel = airStatisticsChannel(widget);
  const request = airStatisticsRequest(widget, track);
  const params = request?.params ?? null;
  const product = useProduct<AirStatisticsReply>(runtime.client, request, active, validateAirStatistics, track?.source);
  return { track, tag, channel, params, options, ...product };
}

/** Channels by their game names, and the regions and theatres the recording names. */
export function useAirStatisticsChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const playbackHour = usePlaybackHour(runtime.clock, widget.time.mode === 'follow');
  const { data, options, track, params } = useAirStatistics(widget, runtime, true);
  const theatres = useAirTheatres(runtime, track, 'air-statistics', params, choiceHour(widget, runtime, track, playbackHour), true);
  const picked = new Set(contentSet(options.regions).map(contentKey));
  return {
    channel: (data?.channels ?? []).map(channel => ({ value: setToken(channel.id), label: `${statisticName(channel.ours)} / ${statisticName(channel.enemy)}` })),
    regions: [...(data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })),
      ...[...picked].filter(id => !data?.regions.some(region => region.id === id)).map(id => ({ value: setToken(id), label: `${id} · not in this recording` }))],
    theatres: theatreChoices(theatres.theatres ?? undefined, contentSet(options.theatres).map(contentKey)),
  };
}

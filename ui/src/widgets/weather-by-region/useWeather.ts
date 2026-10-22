import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import * as requests from '../../dashboard/batch5Requests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { validateWeather } from './weatherData';
import type { WeatherReply } from './weatherData';
import { OPTIONS } from './definition';
import { weatherRequest, weatherShow } from '../../dashboard/widgetRequests';

/** The shown days' prepared weather, sampled once a day, for the chosen view. */
export function useWeather(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const { show, chosen } = weatherShow(widget);
  const request = weatherRequest(widget, track, extent);
  const product = useProduct<WeatherReply>(runtime.client, request, active, validateWeather, track?.source);
  const modifier = show === 'modifiers' ? chosen ?? product.data?.modifiers[0]?.id ?? null : null;
  return { track, options, show, modifier, ...product };
}

/** Regions, theatres and modifiers as the recording and its content name them. */
export function useWeatherChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather',
    params: requests.weatherHistory(track.branch.id, track.branch.lastHour, track.branch.lastHour, 'conditions', null) } : null;
  const { data } = useProduct<WeatherReply>(runtime.client, request, true, validateWeather, track?.source);
  return {
    regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })),
    theatres: (data?.theatres ?? []).map(theatre => ({ value: setToken(theatre.id), label: theatre.name })),
    modifier: (data?.modifiers ?? []).map(modifier => ({ value: setToken(modifier.id), label: modifier.name })),
  };
}

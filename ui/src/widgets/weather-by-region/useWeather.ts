import { useCallback, useEffect, useRef, useState } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { theatreChoices, useAirTheatres } from '../../dashboard/airTheatres.ts';
import { useProduct } from '../../dashboard/products';
import { validateWeather, weatherWindow } from './weatherData.ts';
import type { WeatherReply, WeatherShow } from './weatherData';
import { OPTIONS } from './definition';
import { busyWait } from '../../shell/busy.ts';

/** Column budgets step by this many pixels, so a small resize asks nothing again. */
export const WIDTH_STEP = 256;
export const stepWidth = (pixels: number) => Math.ceil(Math.max(1, pixels) / WIDTH_STEP) * WIDTH_STEP;
/** How long the view rests after a zoom, pan or resize before its days are asked again. */
export const VIEW_SETTLE_MS = 150;

/** What the widget shows: the view and the chosen modifier. */
export function weatherShow(widget: WidgetSpec): { show: WeatherShow; chosen: string | null } {
  const options = resolveOptions(OPTIONS, widget.options);
  const show: WeatherShow = options.show === 'modifiers' || options.show === 'temperature' ? options.show : 'conditions';
  return { show, chosen: options.modifier ? contentKey(String(options.modifier)) : null };
}

/** The shown days, folded into at most `pixels` columns where they do not fit one a column. */
export function weatherRequest(widget: WidgetSpec, track: BoundTrack | undefined, extent: readonly [number, number] | null, pixels: number) {
  const { show, chosen } = weatherShow(widget);
  const window = track && extent ? weatherWindow(track.branch, extent, pixels) : null;
  if (!track || !window || window.first > window.last) return null;
  const asked = show === 'modifiers' ? 'modifier' : show;
  return { sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather', params: {
    branchId: track.branch.id, firstHour: window.first, lastHour: window.last, width: window.width, show: asked,
    ...asked === 'modifier' && chosen ? { modifier: chosen } : {} } };
}

/** The window's width in pixels, until the chart reports its plot's. */
function useWindowWidth(): number {
  const [width, setWidth] = useState(() => stepWidth(globalThis.innerWidth ?? 1));
  useEffect(() => {
    const update = () => setWidth(stepWidth(globalThis.innerWidth));
    globalThis.addEventListener('resize', update);
    return () => globalThis.removeEventListener('resize', update);
  }, []);
  return width;
}

/** The shown days' weather, sampled once a day and folded into at most one column per pixel of the plot (`pixels`,
 * else the window's), for the chosen view. The last reply stays shown while the next loads. */
export function useWeather(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null, pixels: number | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const { show, chosen } = weatherShow(widget);
  const window = useWindowWidth();
  const product = useProduct<WeatherReply>(runtime.client, weatherRequest(widget, track, extent, pixels ?? window), active, validateWeather, track?.source);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, show, chosen]);
  const kept = useRef<{ client: unknown; scope: string; data: WeatherReply } | null>(null);
  if (product.data) kept.current = { client: runtime.client, scope, data: product.data };
  const data = product.data ?? (kept.current?.client === runtime.client && kept.current.scope === scope && !product.error ? kept.current.data : undefined);
  const modifier = show === 'modifiers' ? chosen ?? data?.modifiers[0]?.id ?? null : null;
  return { track, options, show, modifier, ...product, data };
}

/** The weather of the hours the chart shows: `extent` until the chart reports its view, then the hours in view at the
 * plot's width, once the view has settled. */
export function useWeatherInView(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const owner = JSON.stringify([track?.source.id, track?.branch.id, extent?.[0], extent?.[1]]);
  const [view, setView] = useState<{ owner: string; from: number; to: number; pixels: number } | null>(null);
  const settle = useRef<(() => void) | undefined>(undefined);
  useEffect(() => () => settle.current?.(), []);
  const onSummaryView = useCallback((next: { from: number; to: number; width: number }) => {
    settle.current?.();
    settle.current = busyWait(() => setView(previous => {
      const shown = { owner, from: Math.floor(next.from), to: Math.ceil(next.to), pixels: stepWidth(next.width) };
      return previous && previous.owner === shown.owner && previous.from === shown.from && previous.to === shown.to && previous.pixels === shown.pixels ? previous : shown;
    }), VIEW_SETTLE_MS);
  }, [owner]);
  const current = view?.owner === owner ? view : null;
  const asked: [number, number] | null = extent && current ? [Math.max(extent[0], current.from), Math.min(extent[1], current.to)] : null;
  const weather = useWeather(widget, runtime, active, asked && asked[0] <= asked[1] ? asked : extent, current?.pixels ?? null);
  return { ...weather, onSummaryView };
}

/** Regions, theatres and modifiers as the recording and its content name them, from the last day's reply. */
export function useWeatherChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather',
    params: { branchId: track.branch.id, firstHour: track.branch.lastHour, lastHour: track.branch.lastHour, width: 1, show: 'conditions' } } : null;
  const { data } = useProduct<WeatherReply>(runtime.client, request, true, validateWeather, track?.source);
  const air = useAirTheatres(track, track?.branch.lastHour ?? null, true);
  return {
    theatres: theatreChoices(air.theatres, contentSet(resolveOptions(OPTIONS, widget.options).theatres).map(contentKey)),
    regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })),
    modifier: (data?.modifiers ?? []).map(modifier => ({ value: setToken(modifier.id), label: modifier.name })),
  };
}

import { useCampaignGate, useGate, type Gate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { theatreChoices, useAirTheatres } from '../../dashboard/airTheatres.ts';
import { useProduct } from '../../dashboard/products';
import { mergeWeatherTail, validateWeather, weatherTailWindow, weatherWindow } from './weatherData.ts';
import type { WeatherReply, WeatherShow } from './weatherData';
import { OPTIONS } from './definition';
import { busyWait } from '../../shell/busy.ts';
import { productRequest } from '../../dashboard/request.ts';

export const WIDTH_STEP = 256;
export const stepWidth = (pixels: number) => Math.ceil(Math.max(1, pixels) / WIDTH_STEP) * WIDTH_STEP;
export const VIEW_SETTLE_MS = 150;

export function weatherShow(widget: WidgetSpec): { show: WeatherShow; chosen: string | null } {
  const options = resolveOptions(OPTIONS, widget.options);
  const show: WeatherShow = options.show === 'modifiers' || options.show === 'temperature' ? options.show : 'conditions';
  return { show, chosen: options.modifier ? contentKey(String(options.modifier)) : null };
}

function makeWeatherRequest(gate: Pick<Gate, 'span'>, widget: WidgetSpec, track: BoundTrack | undefined,
  window: { first: number; last: number; width: number } | null) {
  const { show, chosen } = weatherShow(widget);
  const span = window && window.first <= window.last ? gate.span(window.first, window.last) : null;
  if (!track || !window || !span) return null;
  const asked = show === 'modifiers' ? 'modifier' : show;
  return productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather', params: {
    branchId: track.branch.id, ...span, width: window.width, show: asked,
    ...asked === 'modifier' && chosen ? { modifier: chosen } : {} } });
}

export function weatherRequests(gate: Pick<Gate, 'span'>, widget: WidgetSpec, track: BoundTrack | undefined, extent: readonly [number, number] | null, pixels: number,
  limit?: number) {
  const baseWindow = track && extent ? weatherWindow(track.branch, extent, pixels) : null;
  const tailWindow = track && extent ? weatherTailWindow(track.branch, extent, pixels, limit) : null;
  return {
    base: makeWeatherRequest(gate, widget, track, baseWindow),
    tail: makeWeatherRequest(gate, widget, track, tailWindow),
  };
}

export function weatherRequest(gate: Pick<Gate, 'span'>, widget: WidgetSpec, track: BoundTrack | undefined, extent: readonly [number, number] | null, pixels: number) {
  return weatherRequests(gate, widget, track, extent, pixels).base;
}

function useWindowWidth(): number {
  const [width, setWidth] = useState(() => stepWidth(globalThis.innerWidth ?? 1));
  useEffect(() => {
    const update = () => setWidth(stepWidth(globalThis.innerWidth));
    globalThis.addEventListener('resize', update);
    return () => globalThis.removeEventListener('resize', update);
  }, []);
  return width;
}

export function useWeather(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null, pixels: number | null,
  limit?: number) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const { show, chosen } = weatherShow(widget);
  const window = useWindowWidth();
  const gate = useGate(runtime, widget, SPEC, active);
  const { base: baseRequest, tail: tailRequest } = weatherRequests(gate, widget, track, extent, pixels ?? window, limit);
  const product = useProduct<WeatherReply>(runtime.client, baseRequest, active, validateWeather, track?.source);
  const tail = useProduct<WeatherReply>(runtime.client, tailRequest, active, validateWeather, track?.source);
  const merged = useMemo(() => product.data ? mergeWeatherTail(product.data, tailRequest ? tail.data : undefined) : undefined,
    [product.data, tailRequest, tail.data]);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, show, chosen]);
  const kept = useRef<{ client: unknown; scope: string; data: WeatherReply } | null>(null);
  if (merged) kept.current = { client: runtime.client, scope, data: merged };
  const error = product.error ?? tail.error;
  const retry = useCallback(() => { product.retry(); tail.retry(); }, [product.retry, tail.retry]);
  const data = merged ?? (kept.current?.client === runtime.client && kept.current.scope === scope && !error ? kept.current.data : undefined);
  const modifier = show === 'modifiers' ? chosen ?? data?.modifiers[0]?.id ?? null : null;
  return { track, options, show, modifier, error, retry, data };
}

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
  const weather = useWeather(widget, runtime, active, asked && asked[0] <= asked[1] ? asked : extent, current?.pixels ?? null, extent?.[1]);
  return { ...weather, onSummaryView };
}

export function useWeatherChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const end = useCampaignGate(runtime, widget).hour;
  const request = track && end !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather',
    params: { branchId: track.branch.id, firstHour: end, lastHour: end, width: 1, show: 'conditions' } }) : null;
  const { data } = useProduct<WeatherReply>(runtime.client, request, true, validateWeather, track?.source);
  const air = useAirTheatres(track, track?.branch.lastHour ?? null, true);
  return {
    theatres: theatreChoices(air.theatres, contentSet(resolveOptions(OPTIONS, widget.options).theatres).map(contentKey)),
    regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })),
    modifier: (data?.modifiers ?? []).map(modifier => ({ value: setToken(modifier.id), label: modifier.name })),
  };
}
